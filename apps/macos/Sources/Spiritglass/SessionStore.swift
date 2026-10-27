import Foundation
import SpiritglassCore
import SwiftUI

@MainActor
final class SessionStore: ObservableObject {
    @Published private(set) var snapshot: Snapshot?
    @Published private(set) var connected = false
    @Published private(set) var connectionMessage = "Connecting to your supervisor…"
    @Published private(set) var pendingActions: Set<String> = []
    @Published var actionError: String?
    let socketPath: String
    private var connection: UnixConnection?
    private var generation = UUID()
    private var nextID = 3
    private var actions: [Int: String] = [:]
    private var actionTimeouts: [Int: Task<Void, Never>] = [:]
    private var reconnect: Task<Void, Never>?
    private var handshakeTimeout: Task<Void, Never>?
    private var receiver: Task<Void, Never>?

    var attention: AttentionSummary { AttentionSummary.resolve(snapshot: snapshot, connected: connected) }

    init() {
        socketPath = ProcessInfo.processInfo.environment["SPIRITGLASS_SOCKET"] ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".spiritglass/run/control.sock").path
    }
    func start() { guard connection == nil else { return }; connect() }
    func stop() {
        generation = UUID()
        reconnect?.cancel(); handshakeTimeout?.cancel()
        receiver?.cancel()
        connection?.close(); connection = nil
        connected = false
        clearActions()
    }
    private func connect() {
        generation = UUID()
        let token = generation
        let transport = UnixConnection(path: socketPath)
        connection = transport
        let events = transport.events()
        receiver = Task { [weak self] in
            for await event in events {
                guard let self, generation == token else { return }
                switch event {
                case .opened: send(id: 1, method: "initialize", params: .object(["protocol_versions": .array([.number(Double(Wire.version))])]))
                case .line(let data): receive(data, token: token)
                case .closed(let reason): disconnected(reason, token: token); return
                }
            }
            self?.disconnected("The connection ended before all messages could be processed.", token: token)
        }
        handshakeTimeout?.cancel()
        handshakeTimeout = Task { [weak self] in
            try? await Task.sleep(for: .seconds(5))
            guard !Task.isCancelled, let self, generation == token, !connected else { return }
            disconnected("The supervisor did not complete its handshake.", token: token)
        }
    }
    private func send(id: Int, method: String, params: JSONValue) {
        let message: JSONValue = .object(["jsonrpc": .string("2.0"), "id": .number(Double(id)), "method": .string(method), "params": params])
        if let data = try? JSONEncoder().encode(message) { connection?.send(data) }
    }
    private func receive(_ data: Data, token: UUID) {
        guard generation == token else { return }
        do {
            let message = try Wire.parseEnvelope(data)
            if let method = message["method"]?.string {
                guard let params = message["params"] else { throw SessionError.invalidMessage }
                switch method {
                case "session.snapshot": try apply(params.decode(Snapshot.self))
                case "state.changed":
                    let update = try params.decode(StateChanged.self)
                    guard snapshot != nil else { throw SessionError.invalidMessage }
                    snapshot?.sources = update.sources
                    snapshot?.health = update.health
                    snapshot?.diagnostic = update.diagnostic
                default: break
                }
                return
            }
            guard let number = message["id"]?.number, let id = Int(exactly: number) else { throw SessionError.invalidMessage }
            if let error = message["error"] {
                let reason = error["message"]?.string ?? "The action failed."
                if id <= 2 { disconnected(reason, token: token) }
                else { finishAction(id); actionError = reason }
                return
            }
            guard let result = message["result"] else { throw SessionError.invalidMessage }
            switch id {
            case 1:
                guard result["protocol_version"]?.number == Double(Wire.version) else { throw SessionError.incompatibleVersion }
                send(id: 2, method: "session.subscribe", params: .object([:]))
            case 2:
                try apply(result.decode(Snapshot.self))
                connected = true
                connectionMessage = "Connected"
                handshakeTimeout?.cancel()
            default: finishAction(id)
            }
        } catch { disconnected("Couldn’t read the supervisor’s message: \(error.localizedDescription)", token: token) }
    }
    private func apply(_ value: Snapshot) throws {
        guard value.protocolVersion == Wire.version else { throw SessionError.incompatibleVersion }
        snapshot = value
    }
    func invoke(widget: SpiritglassCore.Widget) {
        guard connected, snapshot?.health == "ready", let action = widget.action, !pendingActions.contains(widget.id) else { return }
        let id = nextID; nextID += 1
        actions[id] = widget.id
        pendingActions.insert(widget.id)
        actionError = nil
        send(id: id, method: "action.invoke", params: .object(["action": .string(action), "arguments": widget.arguments ?? .object([:])]))
        actionTimeouts[id] = Task { [weak self] in
            try? await Task.sleep(for: .seconds(8))
            guard !Task.isCancelled, let self, actions[id] != nil else { return }
            finishAction(id)
            actionError = "The action timed out. Its completion is unknown."
        }
    }
    private func finishAction(_ id: Int) {
        if let widget = actions.removeValue(forKey: id) { pendingActions.remove(widget) }
        actionTimeouts.removeValue(forKey: id)?.cancel()
    }
    private func clearActions() {
        if !actions.isEmpty { actionError = "The connection closed while an action was running. Its completion is unknown." }
        actions.removeAll(); pendingActions.removeAll()
        for task in actionTimeouts.values { task.cancel() }
        actionTimeouts.removeAll()
    }
    private func disconnected(_ reason: String, token: UUID) {
        guard generation == token else { return }
        generation = UUID()
        connected = false
        connectionMessage = reason
        connection?.close(); connection = nil
        receiver?.cancel()
        handshakeTimeout?.cancel()
        clearActions()
        reconnect?.cancel()
        reconnect = Task { [weak self] in
            try? await Task.sleep(for: .seconds(1))
            guard !Task.isCancelled else { return }
            self?.connect()
        }
    }
}
private enum SessionError: LocalizedError {
    case invalidMessage, incompatibleVersion
    var errorDescription: String? {
        switch self { case .invalidMessage: "Invalid protocol message."; case .incompatibleVersion: "Incompatible protocol version." }
    }
}
