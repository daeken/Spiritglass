import Darwin
import Foundation

public enum ConnectionEvent: Sendable { case opened, line(Data), closed(String) }

/// Blocking POSIX I/O is confined to dedicated dispatch queues. The read worker owns
/// the descriptor; shutdown interrupts it, and close is serialized with all writes.
public final class UnixConnection: @unchecked Sendable {
    private let path: String
    private let lock = NSLock()
    private var descriptor: Int32 = -1
    private var stopped = false
    private let readQueue = DispatchQueue(label: "dev.spiritglass.socket.read", qos: .userInitiated)
    private let writeQueue = DispatchQueue(label: "dev.spiritglass.socket.write", qos: .userInitiated)

    public init(path: String) { self.path = path }
    public func events() -> AsyncStream<ConnectionEvent> {
        AsyncStream(bufferingPolicy: .bufferingOldest(64)) { continuation in
            start(onOpen: { continuation.yield(.opened) }, onLine: { [weak self] line in
                if case .dropped = continuation.yield(.line(line)) {
                    self?.close()
                    continuation.finish()
                }
            }, onClose: {
                continuation.yield(.closed($0))
                continuation.finish()
            })
            continuation.onTermination = { [weak self] _ in self?.close() }
        }
    }
    private func start(onOpen: @escaping @Sendable () -> Void, onLine: @escaping @Sendable (Data) -> Void, onClose: @escaping @Sendable (String) -> Void) {
        readQueue.async { [self] in
            let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
            guard fd >= 0 else { onClose("Couldn’t create a local connection."); return }
            lock.lock()
            if stopped { lock.unlock(); Darwin.close(fd); return }
            descriptor = fd
            lock.unlock()
            defer {
                lock.lock()
                descriptor = -1
                Darwin.close(fd)
                lock.unlock()
            }
            var address = sockaddr_un()
            address.sun_family = sa_family_t(AF_UNIX)
            let bytes = Array(path.utf8) + [0]
            guard bytes.count <= MemoryLayout.size(ofValue: address.sun_path) else { onClose("The local socket path is too long."); return }
            withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: bytes) }
            address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
            var noSignal: Int32 = 1
            _ = setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &noSignal, socklen_t(MemoryLayout<Int32>.size))
            var writeTimeout = timeval(tv_sec: 2, tv_usec: 0)
            _ = setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &writeTimeout, socklen_t(MemoryLayout<timeval>.size))
            let result = withUnsafePointer(to: &address) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
            }
            guard result == 0 else { onClose("Waiting for the supervisor."); return }
            onOpen()
            var pending = Data()
            var buffer = [UInt8](repeating: 0, count: 16_384)
            while true {
                let count = Darwin.read(fd, &buffer, buffer.count)
                if count < 0 && errno == EINTR { continue }
                guard count > 0 else { onClose("The supervisor disconnected."); return }
                pending.append(contentsOf: buffer.prefix(count))
                while let newline = pending.firstIndex(of: 10) {
                    let line = Data(pending[..<newline])
                    guard line.count <= Wire.maximumMessageBytes else { onClose("The supervisor sent an oversized message."); return }
                    pending.removeSubrange(...newline)
                    onLine(line)
                }
                guard pending.count <= Wire.maximumMessageBytes else { onClose("The supervisor sent an oversized message."); return }
            }
        }
    }
    public func send(_ data: Data) {
        guard data.count <= Wire.maximumMessageBytes else { close(); return }
        writeQueue.async { [self] in
            lock.lock()
            defer { lock.unlock() }
            guard !stopped, descriptor >= 0 else { return }
            var line = data
            line.append(10)
            line.withUnsafeBytes { bytes in
                var written = 0
                while written < bytes.count {
                    let count = Darwin.write(descriptor, bytes.baseAddress!.advanced(by: written), bytes.count - written)
                    if count < 0 && errno == EINTR { continue }
                    if count <= 0 { Darwin.shutdown(descriptor, SHUT_RDWR); return }
                    written += count
                }
            }
        }
    }
    public func close() {
        lock.lock()
        stopped = true
        if descriptor >= 0 { Darwin.shutdown(descriptor, SHUT_RDWR) }
        lock.unlock()
    }
    deinit { close() }
}
