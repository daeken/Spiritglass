import Foundation

public enum AttentionSeverity: String, CaseIterable, Comparable, Sendable {
    case clear, notice, urgent, emergency

    private var rank: Int {
        switch self { case .clear: 0; case .notice: 1; case .urgent: 2; case .emergency: 3 }
    }
    public static func < (lhs: Self, rhs: Self) -> Bool { lhs.rank < rhs.rank }
    public var title: String { rawValue.capitalized }
}

public struct AttentionItem: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let severity: AttentionSeverity?
    public let availability: Availability

    public var detail: String {
        switch availability {
        case .stale: "Last known \(severity?.rawValue ?? "unknown") · stale"
        case .loading: "Loading"
        case .unavailable: "Unavailable"
        case .ready: severity?.title ?? "Unknown"
        }
    }
}

/// Until modules expose persisted alerts, the menu bar summarizes the dashboard's
/// declared status components. It never assumes source field names or module IDs.
public struct AttentionSummary: Equatable, Sendable {
    public let severity: AttentionSeverity?
    public let hasUnknown: Bool
    public let items: [AttentionItem]

    public var title: String {
        guard let severity else { return "Monitoring unknown" }
        let title = severity == .clear ? "All clear" : severity.title
        return hasUnknown ? "\(title) · monitoring incomplete" : title
    }

    public static func resolve(snapshot: Snapshot?, connected: Bool) -> Self {
        var items: [AttentionItem] = []
        var pending = snapshot?.view.map { [$0] } ?? []
        while let widget = pending.popLast() {
            if widget.type == "status" {
                let value = BindingResolver.resolve(widget.value, snapshot: snapshot, connected: connected)
                var availability = value.availability
                if !connected || snapshot?.health != "ready" {
                    availability = value.value == nil ? .unavailable : .stale
                }
                items.append(AttentionItem(
                    id: widget.id, label: widget.label ?? "Status",
                    severity: value.value?.string.flatMap(AttentionSeverity.init(rawValue:)),
                    availability: availability
                ))
            }
            pending.append(contentsOf: (widget.children ?? []).reversed())
        }
        let unknown = !connected || snapshot?.health != "ready" || items.isEmpty ||
            items.contains { $0.availability != .ready || $0.severity == nil }
        var severity = items.compactMap(\.severity).max()
        // Retain known warnings during outages, but never show stale clear as green.
        if unknown && severity == .clear { severity = nil }
        return Self(severity: severity, hasUnknown: unknown, items: items)
    }

    public static func resolve(catalog: SessionSnapshot?, connected: Bool) -> Self {
        guard let catalog else { return Self(severity: nil, hasUnknown: true, items: []) }
        var items: [AttentionItem] = []
        var unknown = !connected || !catalog.diagnostics.isEmpty
        for module in catalog.orderedModules where module.enabled && module.phase != "dormant" {
            let summary = resolve(snapshot: module.state, connected: connected)
            unknown = unknown || summary.hasUnknown
            if summary.items.isEmpty {
                items.append(AttentionItem(id: "\(module.id)::health", label: module.name, severity: nil, availability: .unavailable))
            }
            items += summary.items.map { AttentionItem(id: "\(module.id)::\($0.id)", label: "\(module.name) · \($0.label)", severity: $0.severity, availability: $0.availability) }
            if module.reloadError != nil {
                items.append(AttentionItem(id: "\(module.id)::@replacement", label: "\(module.name) · replacement failed", severity: .notice, availability: .ready))
            }
        }
        for (index, diagnostic) in catalog.diagnostics.enumerated() {
            items.append(AttentionItem(id: "installation-\(index)", label: diagnostic.message, severity: .notice, availability: .ready))
        }
        unknown = unknown || items.isEmpty
        var severity = items.compactMap(\.severity).max()
        if unknown && severity == .clear { severity = nil }
        return Self(severity: severity, hasUnknown: unknown, items: items)
    }
}
