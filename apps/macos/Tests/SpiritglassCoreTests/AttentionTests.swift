import Foundation
import Testing
@testable import SpiritglassCore

private func attentionSnapshot(_ statuses: [String], health: String = "ready", sourceStatus: String = "available") throws -> Snapshot {
    let values = Dictionary(uniqueKeysWithValues: statuses.enumerated().map { ("status-\($0.offset)", $0.element) })
    let widgets = statuses.indices.map { index -> [String: Any] in [
        "id": "indicator-\(index)", "type": "status", "label": "Check \(index)",
        "value": ["kind": "binding", "source": "monitor", "path": ["status-\(index)"]],
    ] }
    let value: [String: Any] = [
        "protocol_version": 1, "health": health,
        "module": ["id": "test", "name": "Test module", "description": "", "view_id": "main"],
        "view": ["id": "dashboard", "type": "v_stack", "children": widgets],
        "sources": ["monitor": ["value": values, "status": sourceStatus, "revision": 1, "updated_at_ms": 0]],
    ]
    return try Wire.decoder().decode(Snapshot.self, from: JSONSerialization.data(withJSONObject: value))
}

@Test func trafficLightFollowsWorstDeclaredStatus() throws {
    for severity in AttentionSeverity.allCases {
        let snapshot = try attentionSnapshot(["clear", severity.rawValue])
        let result = AttentionSummary.resolve(snapshot: snapshot, connected: true)
        #expect(result.severity == severity)
        #expect(!result.hasUnknown)
        #expect(result.items.map(\.id) == ["indicator-0", "indicator-1"])
    }
}

@Test func incompleteMonitoringNeverShowsGreen() throws {
    let clear = try attentionSnapshot(["clear"])
    let disconnected = AttentionSummary.resolve(snapshot: clear, connected: false)
    #expect(disconnected.severity == nil)
    #expect(disconnected.hasUnknown)
    #expect(disconnected.items.first?.availability == .stale)

    for snapshot in [
        try attentionSnapshot(["clear"], health: "unavailable"),
        try attentionSnapshot(["clear"], sourceStatus: "stale"),
        try attentionSnapshot(["clear", "unknown"]),
        try attentionSnapshot(["clear", "unexpected-future-value"]),
        try attentionSnapshot([]),
    ] {
        let result = AttentionSummary.resolve(snapshot: snapshot, connected: true)
        #expect(result.severity == nil)
        #expect(result.hasUnknown)
    }
    #expect(AttentionSummary.resolve(snapshot: nil, connected: false).severity == nil)
}

@Test func outagesRetainKnownWarningsAlongsideUnknownMonitoring() throws {
    let snapshot = try attentionSnapshot(["clear", "emergency"])
    let result = AttentionSummary.resolve(snapshot: snapshot, connected: false)
    #expect(result.severity == .emergency)
    #expect(result.hasUnknown)
    #expect(result.title == "Emergency · monitoring incomplete")
    #expect(result.items.last?.detail == "Last known emergency · stale")

    let partial = AttentionSummary.resolve(snapshot: try attentionSnapshot(["urgent", "unknown"]), connected: true)
    #expect(partial.severity == .urgent)
    #expect(partial.hasUnknown)
}

@Test func missingBindingsAndLiteralStatusesRespectConnectionHealth() throws {
    var snapshot = try attentionSnapshot(["clear"])
    snapshot.sources.removeAll()
    #expect(AttentionSummary.resolve(snapshot: snapshot, connected: true).severity == nil)
    #expect(AttentionSummary.resolve(snapshot: snapshot, connected: true).hasUnknown)

    let literal = Data(#"{"protocol_version":1,"health":"ready","sources":{},"view":{"id":"constant","type":"status","label":"A constant status","value":{"kind":"literal","value":"clear"}}}"#.utf8)
    let fixed = try Wire.decoder().decode(Snapshot.self, from: literal)
    #expect(AttentionSummary.resolve(snapshot: fixed, connected: true).severity == .clear)
    #expect(AttentionSummary.resolve(snapshot: fixed, connected: false).severity == nil)
}
