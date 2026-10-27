import Foundation
import Testing
@testable import SpiritglassCore

private func sessionJSON() throws -> [String: Any] {
    let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent()
    return try #require(JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("protocol/fixtures/session.snapshot.json"))) as? [String: Any])
}
private func decodeSession(_ value: [String: Any]) throws -> SessionSnapshot {
    try Wire.decoder().decode(SessionSnapshot.self, from: JSONSerialization.data(withJSONObject: value))
}

@Test func decodesSharedMultiModuleSessionAndAggregatesAttention() throws {
    let session = try decodeSession(sessionJSON())
    try session.validate()
    #expect(session.protocolVersion == Wire.version)
    #expect(session.modules.count == 2)
    let first = try #require(session.modules["dev.spiritglass.example"])
    let second = try #require(session.modules["dev.spiritglass.project"])
    #expect(first.state.sources["overview"]?.value["count"]?.number == 0)
    #expect(second.state.sources["project"]?.value["changed"]?.number == 2)
    let attention = AttentionSummary.resolve(catalog: session, connected: true)
    #expect(attention.severity == .notice)
    #expect(!attention.hasUnknown)
    #expect(Set(attention.items.map(\.id)).count == attention.items.count)
    #expect(attention.items.contains { $0.label.contains("Project watch") })
}

@Test func sourceOnlyUpdatesAreAtomicAndScopedToTheirModule() throws {
    var session = try decodeSession(sessionJSON())
    let firstID = "dev.spiritglass.example"
    let secondID = "dev.spiritglass.project"
    let before = session.modules[secondID]?.state.sources["project"]?.value
    var updates = session.modules.mapValues { StateChanged(sources: $0.state.sources, health: $0.state.health, diagnostic: $0.state.diagnostic) }
    let state = SourceState(value: .object(["count": .number(7), "uptime": .number(1), "status": .string("notice"), "message": .string("Changed")]), status: "available", revision: 2, updatedAtMs: 1)
    updates[firstID] = StateChanged(sources: ["overview": state], health: "ready", diagnostic: nil)
    try session.apply(SessionStateChanged(modules: updates))
    #expect(session.modules[firstID]?.state.sources["overview"]?.value["count"]?.number == 7)
    #expect(session.modules[secondID]?.state.sources["project"]?.value == before)
    #expect(session.modules[firstID]?.state.view?.id == "dashboard")

    updates["unexpected"] = updates[firstID]
    #expect(throws: (any Error).self) { try session.apply(SessionStateChanged(modules: updates)) }
    #expect(session.modules[firstID]?.state.sources["overview"]?.revision == 2)
}

@Test func disabledAndDormantModulesDoNotContributeAttention() throws {
    for phase in ["disabled", "dormant"] {
        var value = try sessionJSON()
        var modules = try #require(value["modules"] as? [String: [String: Any]])
        modules["dev.spiritglass.project"]?["phase"] = phase
        modules["dev.spiritglass.project"]?["enabled"] = phase != "disabled"
        value["modules"] = modules
        let attention = AttentionSummary.resolve(catalog: try decodeSession(value), connected: true)
        #expect(attention.severity == .clear)
        #expect(!attention.hasUnknown)
        #expect(attention.items.count == 1)
    }
}

@Test func catalogRejectsMismatchedIdentitiesAndInstallationErrorsAreVisible() throws {
    var value = try sessionJSON()
    var modules = try #require(value["modules"] as? [String: [String: Any]])
    modules["dev.spiritglass.project"]?["id"] = "another-module"
    value["modules"] = modules
    let invalid = try decodeSession(value)
    #expect(throws: (any Error).self) { try invalid.validate() }

    value = try sessionJSON()
    value["diagnostics"] = [["at_ms": 0, "message": "Invalid manifest"]]
    let attention = AttentionSummary.resolve(catalog: try decodeSession(value), connected: true)
    #expect(attention.hasUnknown)
    #expect(attention.severity == .notice)
    #expect(attention.items.contains { $0.label == "Invalid manifest" })
}

@Test func syntheticAttentionItemsCannotCollideWithValidWidgetIDs() throws {
    var value = try sessionJSON()
    var modules = try #require(value["modules"] as? [String: [String: Any]])
    var example = try #require(modules["dev.spiritglass.example"])
    var state = try #require(example["state"] as? [String: Any])
    state["view"] = ["id": "replacement", "type": "status", "label": "Replacement service", "value": ["kind": "literal", "value": "clear"]]
    example["state"] = state
    example["reload_error"] = "Failed candidate"
    modules["dev.spiritglass.example"] = example
    value["modules"] = modules
    let attention = AttentionSummary.resolve(catalog: try decodeSession(value), connected: true)
    #expect(Set(attention.items.map(\.id)).count == attention.items.count)
    #expect(attention.items.filter { $0.label.contains("replacement failed") }.count == 1)
}
