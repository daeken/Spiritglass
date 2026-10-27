import Foundation

extension Wire {
    /// Validate the envelope before any application state is changed. Method payloads
    /// are subsequently decoded into their specific wire types by the session.
    public static func parseEnvelope(_ data: Data) throws -> JSONValue {
        guard data.count <= maximumMessageBytes else { throw EnvelopeError.invalid }
        let value = try JSONDecoder().decode(JSONValue.self, from: data)
        guard case .object(let fields) = value, fields["jsonrpc"]?.string == "2.0" else { throw EnvelopeError.invalid }
        let keys = Set(fields.keys)
        if let method = fields["method"]?.string {
            guard !method.isEmpty, keys.isSubset(of: ["jsonrpc", "method", "params"]) else { throw EnvelopeError.invalid }
            if let params = fields["params"] {
                switch params { case .object, .array: break; default: throw EnvelopeError.invalid }
            }
        } else {
            let success = fields["result"] != nil
            guard keys == Set(["jsonrpc", "id", success ? "result" : "error"]) else { throw EnvelopeError.invalid }
            switch fields["id"] {
            case .number(let id): guard id >= 0, id.rounded() == id else { throw EnvelopeError.invalid }
            case .string: break
            case .null: guard !success else { throw EnvelopeError.invalid }
            default: throw EnvelopeError.invalid
            }
            if !success {
                guard case .object(let error) = fields["error"], Set(error.keys) == ["code", "message"],
                      let code = error["code"]?.number, Int32(exactly: code) != nil,
                      error["message"]?.string != nil else { throw EnvelopeError.invalid }
            }
        }
        return value
    }
}
private enum EnvelopeError: LocalizedError {
    case invalid
    var errorDescription: String? { "Invalid JSON-RPC envelope." }
}
