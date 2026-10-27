import Foundation

public enum JSONValue: Codable, Equatable, Sendable {
    case string(String), number(Double), bool(Bool), object([String: JSONValue]), array([JSONValue]), null

    public init(from decoder: any Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let bool = try? value.decode(Bool.self) { self = .bool(bool) }
        else if let number = try? value.decode(Double.self) { self = .number(number) }
        else if let string = try? value.decode(String.self) { self = .string(string) }
        else if let object = try? value.decode([String: JSONValue].self) { self = .object(object) }
        else { self = .array(try value.decode([JSONValue].self)) }
    }
    public func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .string(let value): try container.encode(value)
        case .number(let value): try container.encode(value)
        case .bool(let value): try container.encode(value)
        case .object(let value): try container.encode(value)
        case .array(let value): try container.encode(value)
        case .null: try container.encodeNil()
        }
    }
    public var string: String? { if case .string(let value) = self { value } else { nil } }
    public var number: Double? { if case .number(let value) = self { value } else { nil } }
    public subscript(key: String) -> JSONValue? { if case .object(let value) = self { value[key] } else { nil } }
    public func at(path: [String]) -> JSONValue? {
        var current = self
        for part in path {
            switch current {
            case .object(let value): guard let next = value[part] else { return nil }; current = next
            case .array(let values): guard let index = Int(part), values.indices.contains(index) else { return nil }; current = values[index]
            default: return nil
            }
        }
        return current
    }
    public func decode<T: Decodable>(_ type: T.Type) throws -> T {
        try Wire.decoder().decode(type, from: JSONEncoder().encode(self))
    }
}

public enum Wire {
    public static let version = 2
    public static let moduleVersion = 1
    public static let maximumMessageBytes = 1_048_576
    public static func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }
}

public struct ModuleDescription: Decodable, Sendable {
    public let id: String
    public let name: String
    public let description: String
    public let viewId: String
}
public struct Property: Decodable, Sendable {
    public let kind: String
    public let value: JSONValue?
    public let source: String?
    public let path: [String]?
}
public struct Widget: Decodable, Identifiable, Sendable {
    public let id: String
    public let type: String
    public let children: [Widget]?
    public let title: String?
    public let label: String?
    public let value: Property?
    public let unit: String?
    public let action: String?
    public let arguments: JSONValue?
}
public struct SourceState: Decodable, Sendable {
    public let value: JSONValue
    public var status: String
    public let revision: UInt64
    public let updatedAtMs: UInt64
}
public struct Snapshot: Decodable, Sendable {
    public let protocolVersion: Int
    public let module: ModuleDescription?
    public let view: Widget?
    public var sources: [String: SourceState]
    public var health: String
    public var diagnostic: String?
}
public struct StateChanged: Decodable, Sendable {
    public let sources: [String: SourceState]
    public let health: String
    public let diagnostic: String?
}
public struct Diagnostic: Decodable, Sendable, Identifiable {
    public let atMs: UInt64
    public let message: String
    public var id: String { "\(atMs):\(message)" }
}
public struct ManagedModule: Decodable, Sendable, Identifiable {
    public let id: String
    public let name: String
    public let lifecycle: String
    public let enabled: Bool
    public let phase: String
    public let generation: UInt64
    public let pid: Int?
    public let restartCount: UInt32
    public let reloading: Bool
    public let reloadError: String?
    public let history: [Diagnostic]
    public var state: Snapshot
}
public struct SessionSnapshot: Decodable, Sendable {
    public let protocolVersion: Int
    public var modules: [String: ManagedModule]
    public let diagnostics: [Diagnostic]
    public var orderedModules: [ManagedModule] { modules.values.sorted { ($0.name, $0.id) < ($1.name, $1.id) } }
    public func validate() throws {
        guard protocolVersion == Wire.version, modules.count <= 32,
              modules.allSatisfy({ $0.key == $0.value.id && $0.value.state.protocolVersion == Wire.moduleVersion }) else {
            throw CatalogError.invalidSnapshot
        }
    }
    public mutating func apply(_ update: SessionStateChanged) throws {
        guard Set(update.modules.keys) == Set(modules.keys) else { throw CatalogError.invalidSnapshot }
        for (id, state) in update.modules {
            modules[id]?.state.sources = state.sources
            modules[id]?.state.health = state.health
            modules[id]?.state.diagnostic = state.diagnostic
        }
    }
}
private enum CatalogError: LocalizedError {
    case invalidSnapshot
    var errorDescription: String? { "Invalid module catalog or update." }
}
public struct SessionStateChanged: Decodable, Sendable {
    public let modules: [String: StateChanged]
}
public struct ActionCompletion: Decodable, Sendable {
    public let generation: UInt64
    public let activeGeneration: UInt64?
    public let value: JSONValue
}
public enum Availability: Equatable, Sendable { case ready, loading, unavailable, stale }
public struct ResolvedValue: Sendable {
    public let value: JSONValue?
    public let availability: Availability
}
public enum BindingResolver {
    public static func resolve(_ property: Property?, snapshot: Snapshot?, connected: Bool) -> ResolvedValue {
        guard let property else { return .init(value: nil, availability: .unavailable) }
        if property.kind == "literal" { return .init(value: property.value, availability: property.value == nil ? .unavailable : .ready) }
        guard property.kind == "binding", let source = property.source, let path = property.path else { return .init(value: nil, availability: .unavailable) }
        guard let snapshot, let state = snapshot.sources[source] else { return .init(value: nil, availability: snapshot?.health == "starting" ? .loading : .unavailable) }
        guard let value = state.value.at(path: path) else { return .init(value: nil, availability: .unavailable) }
        let stale = !connected || snapshot.health != "ready" || state.status != "available"
        return .init(value: value, availability: stale ? .stale : .ready)
    }
}
