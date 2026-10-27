import Foundation
import SpiritglassCore
import SwiftUI

@MainActor
final class SessionStore: ObservableObject {
    @Published private(set) var catalog: SessionSnapshot?
    @Published private(set) var selectedModuleID: String?
    @Published private(set) var connected = false
    @Published private(set) var connectionMessage = "Connecting to your supervisor…"
    @Published private(set) var pendingActions: Set<String> = []
    @Published private(set) var busyModules: Set<String> = []
    @Published var actionError: String?
    let socketPath: String
    private var connection: UnixConnection?
    private var generation = UUID()
    private var nextID = 3
    private var actions: [Int: (key: String, moduleID: String, generation: UInt64)] = [:]
    private var controls: [Int: String] = [:]
    private var timeouts: [Int: Task<Void, Never>] = [:]
    private var reconnect: Task<Void, Never>?
    private var handshakeTimeout: Task<Void, Never>?
    private var receiver: Task<Void, Never>?

    var selectedModule: ManagedModule? { selectedModuleID.flatMap { catalog?.modules[$0] } }
    var snapshot: Snapshot? { selectedModule?.state }
    var attention: AttentionSummary { AttentionSummary.resolve(catalog: catalog, connected: connected) }

    init() {
        socketPath = ProcessInfo.processInfo.environment["SPIRITGLASS_SOCKET"] ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".spiritglass/run/control.sock").path
    }
    func start() { guard connection == nil else { return }; connect() }
    func stop() {
        generation = UUID()
        reconnect?.cancel(); handshakeTimeout?.cancel(); receiver?.cancel()
        connection?.close(); connection = nil
        connected = false
        clearRequests()
    }
    func selectModule(_ id: String?) {
        selectedModuleID = id
        if let id, catalog?.modules[id]?.phase == "dormant" { control("module.activate", moduleID: id) }
    }
    func isPending(_ widgetID: String) -> Bool {
        pendingActions.contains("\(selectedModuleID ?? "")::\(widgetID)")
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
                case "session.snapshot": try apply(params.decode(SessionSnapshot.self))
                case "state.changed":
                    let update = try params.decode(SessionStateChanged.self)
                    guard var next = catalog else { throw SessionError.invalidMessage }
                    try next.apply(update)
                    catalog = next
                default: break
                }
                return
            }
            guard let number = message["id"]?.number, let id = Int(exactly: number) else { throw SessionError.invalidMessage }
            if let error = message["error"] {
                let reason = error["message"]?.string ?? "The request failed."
                if id <= 2 { disconnected(reason, token: token) }
                else { finish(id); actionError = reason }
                return
            }
            guard let result = message["result"] else { throw SessionError.invalidMessage }
            switch id {
            case 1:
                guard result["protocol_version"]?.number == Double(Wire.version) else { throw SessionError.incompatibleVersion }
                send(id: 2, method: "session.subscribe", params: .object([:]))
            case 2:
                try apply(result.decode(SessionSnapshot.self))
                connected = true
                connectionMessage = "Connected"
                handshakeTimeout?.cancel()
            default:
                if actions[id] != nil {
                    let completion = try result.decode(ActionCompletion.self)
                    if completion.activeGeneration != completion.generation {
                        actionError = "The action completed on a module version that is no longer active."
                    }
                }
                finish(id)
            }
        } catch { disconnected("Couldn’t read the supervisor’s message: \(error.localizedDescription)", token: token) }
    }
    private func apply(_ value: SessionSnapshot) throws {
        guard value.protocolVersion == Wire.version else { throw SessionError.incompatibleVersion }
        try value.validate()
        catalog = value
        if let id = selectedModuleID, value.modules[id] == nil { selectedModuleID = nil }
    }
    func invoke(widget: SpiritglassCore.Widget) {
        guard connected, let module = selectedModule, module.phase == "ready", let action = widget.action, !isPending(widget.id) else { return }
        let id = nextID; nextID += 1
        let key = "\(module.id)::\(widget.id)"
        actions[id] = (key, module.id, module.generation)
        pendingActions.insert(key)
        actionError = nil
        send(id: id, method: "action.invoke", params: .object(["module_id": .string(module.id), "action": .string(action), "arguments": widget.arguments ?? .object([:])]))
        timeout(id)
    }
    func control(_ method: String, moduleID: String) {
        guard connected, !busyModules.contains(moduleID) else { return }
        let id = nextID; nextID += 1
        controls[id] = moduleID; busyModules.insert(moduleID); actionError = nil
        send(id: id, method: method, params: .object(["module_id": .string(moduleID)]))
        timeout(id)
    }
    private func timeout(_ id: Int) {
        timeouts[id] = Task { [weak self] in
            try? await Task.sleep(for: .seconds(8))
            guard !Task.isCancelled, let self else { return }
            finish(id); actionError = "The request timed out. Its completion is unknown."
        }
    }
    private func finish(_ id: Int) {
        if let request = actions.removeValue(forKey: id) { pendingActions.remove(request.key) }
        if let module = controls.removeValue(forKey: id) { busyModules.remove(module) }
        timeouts.removeValue(forKey: id)?.cancel()
    }
    private func clearRequests() {
        if !actions.isEmpty { actionError = "The connection closed while an action was running. Its completion is unknown." }
        actions.removeAll(); controls.removeAll(); pendingActions.removeAll(); busyModules.removeAll()
        for task in timeouts.values { task.cancel() }
        timeouts.removeAll()
    }
    private func disconnected(_ reason: String, token: UUID) {
        guard generation == token else { return }
        generation = UUID()
        connected = false; connectionMessage = reason
        connection?.close(); connection = nil
        receiver?.cancel(); handshakeTimeout?.cancel(); clearRequests(); reconnect?.cancel()
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
        switch self { case .invalidMessage: "Invalid protocol message."; case .incompatibleVersion: "Incompatible client protocol. Update and restart the supervisor." }
    }
}
