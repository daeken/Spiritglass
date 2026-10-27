import Foundation
import Testing
@testable import SpiritglassCore

private func fixture() throws -> Snapshot {
    let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent()
    return try Wire.decoder().decode(Snapshot.self, from: Data(contentsOf: root.appendingPathComponent("protocol/fixtures/demo.snapshot.json")))
}
private func node(_ id: String, in widget: Widget) -> Widget? {
    if widget.id == id { return widget }
    return widget.children?.compactMap { node(id, in: $0) }.first
}

@Test func decodesActualRustSnapshotAndResolvesBindings() throws {
    let snapshot = try fixture()
    #expect(snapshot.protocolVersion == Wire.version)
    #expect(snapshot.module?.id == "dev.spiritglass.example")
    let root = try #require(snapshot.view)
    #expect(root.type == "v_stack")
    let metric = try #require(node("count", in: root))
    let resolved = BindingResolver.resolve(metric.value, snapshot: snapshot, connected: true)
    #expect(resolved.availability == .ready)
    #expect(resolved.value?.number == 0)
    #expect(snapshot.sources["overview"]?.updatedAtMs == 0)
}

@Test func disconnectedAndFailedModulesNeverResolveAsLive() throws {
    var snapshot = try fixture()
    let root = try #require(snapshot.view)
    let status = try #require(node("status", in: root))
    let disconnected = BindingResolver.resolve(status.value, snapshot: snapshot, connected: false)
    #expect(disconnected.value?.string == "clear")
    #expect(disconnected.availability == .stale)
    snapshot.health = "unavailable"
    #expect(BindingResolver.resolve(status.value, snapshot: snapshot, connected: true).availability == .stale)
    snapshot.sources.removeAll()
    #expect(BindingResolver.resolve(status.value, snapshot: snapshot, connected: true).availability == .unavailable)
    snapshot.health = "starting"
    #expect(BindingResolver.resolve(status.value, snapshot: snapshot, connected: true).availability == .loading)
}

@Test func unknownWidgetsDecodeWithoutLosingTheirSiblings() throws {
    let data = Data(#"{"id":"root","type":"v_stack","children":[{"id":"new","type":"future_widget","future_property":true},{"id":"label","type":"text","value":{"kind":"literal","value":"Still here"}}]}"#.utf8)
    let root = try Wire.decoder().decode(Widget.self, from: data)
    #expect(root.children?.count == 2)
    #expect(root.children?.first?.type == "future_widget")
    #expect(BindingResolver.resolve(root.children?.last?.value, snapshot: nil, connected: false).value?.string == "Still here")
}

@Test func bindingPathsTraverseArraysWithoutExpressions() throws {
    let value: JSONValue = .object(["items": .array([.object(["name": .string("first")])])])
    #expect(value.at(path: ["items", "0", "name"]) == .string("first"))
    #expect(value.at(path: ["items", "-1"]) == nil)
    #expect(value.at(path: ["items", "1"]) == nil)
    #expect(value.at(path: ["missing"]) == nil)
    let encoded = try JSONEncoder().encode(value)
    #expect(try JSONDecoder().decode(JSONValue.self, from: encoded) == value)
}

@Test func rejectsMalformedServerEnvelopesBeforeApplyingState() throws {
    let invalid = [
        #"{"jsonrpc":"1.0","id":1,"result":null}"#,
        #"{"jsonrpc":"2.0","id":1,"result":1,"error":{"code":1,"message":"error"}}"#,
        #"{"jsonrpc":"2.0","method":"state.changed","params":5}"#,
        #"{"jsonrpc":"2.0","id":1}"#,
        #"{"jsonrpc":"2.0","id":null,"result":null}"#,
        #"{"jsonrpc":"2.0","id":1,"error":{"code":"bad","message":"error"}}"#,
        "[]",
    ]
    for line in invalid {
        #expect(throws: (any Error).self) { try Wire.parseEnvelope(Data(line.utf8)) }
    }
    let response = try Wire.parseEnvelope(Data(#"{"jsonrpc":"2.0","id":1,"result":null}"#.utf8))
    #expect(response["result"] == .null)
    let notification = try Wire.parseEnvelope(Data(#"{"jsonrpc":"2.0","method":"state.changed","params":{}}"#.utf8))
    #expect(notification["method"]?.string == "state.changed")
}
