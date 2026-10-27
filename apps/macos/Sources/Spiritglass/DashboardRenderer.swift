import SpiritglassCore
import SwiftUI

struct DashboardRenderer: View {
    let widget: SpiritglassCore.Widget
    @EnvironmentObject private var session: SessionStore

    private var resolved: ResolvedValue { BindingResolver.resolve(widget.value, snapshot: session.snapshot, connected: session.connected) }
    private var children: [SpiritglassCore.Widget] { widget.children ?? [] }

    var body: some View {
        content.id(widget.id)
    }
    @ViewBuilder private var content: some View {
        switch widget.type {
        case "v_stack":
            VStack(alignment: .leading, spacing: 22) { childViews }
                .frame(maxWidth: .infinity, alignment: .leading)
        case "h_stack":
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .top, spacing: 16) { childViews }
                VStack(alignment: .leading, spacing: 16) { childViews }
            }
        case "card":
            VStack(alignment: .leading, spacing: 18) {
                if let title = widget.title { Text(title).font(.headline) }
                childViews
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(24)
            .background(.background, in: RoundedRectangle(cornerRadius: 18))
            .overlay(RoundedRectangle(cornerRadius: 18).stroke(.primary.opacity(0.07)))
        case "text":
            VStack(alignment: .leading, spacing: 4) {
                Text(resolved.value?.string ?? placeholder).font(.body).foregroundStyle(.secondary).textSelection(.enabled)
                availabilityNote
            }
        case "metric":
            VStack(alignment: .leading, spacing: 10) {
                Text(widget.label ?? "Metric").font(.subheadline).foregroundStyle(.secondary)
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    if let value = resolved.value?.number {
                        Text(value, format: .number.precision(.fractionLength(0...2)))
                            .font(.system(size: 40, weight: .medium, design: .rounded)).monospacedDigit()
                    } else { Text(placeholder).font(.system(size: 40, weight: .medium, design: .rounded)) }
                    if let unit = widget.unit { Text(unit).font(.callout).foregroundStyle(.secondary) }
                }
                availabilityNote
            }
            .frame(maxWidth: .infinity, minHeight: 110, alignment: .leading)
            .padding(24)
            .background(.background, in: RoundedRectangle(cornerRadius: 18))
            .overlay(RoundedRectangle(cornerRadius: 18).stroke(.primary.opacity(0.07)))
        case "status":
            HStack(spacing: 10) {
                Image(systemName: statusSymbol).foregroundStyle(statusColor).font(.body.weight(.semibold))
                Text(widget.label ?? "Status").font(.subheadline.weight(.medium))
                Spacer()
                Text(statusLabel).font(.subheadline).foregroundStyle(.secondary)
            }
            .accessibilityElement(children: .combine)
        case "button":
            Button { session.invoke(widget: widget) } label: {
                HStack(spacing: 8) {
                    if session.pendingActions.contains(widget.id) { ProgressView().controlSize(.mini) }
                    Text(widget.label ?? "Run action")
                }
            }
            .buttonStyle(.bordered).controlSize(.large)
            .disabled(!session.connected || session.snapshot?.health != "ready" || session.pendingActions.contains(widget.id))
        case "divider": Divider()
        default:
            Label("Unsupported component: \(widget.type)", systemImage: "square.dashed")
                .font(.callout).foregroundStyle(.secondary).padding()
        }
    }
    private var childViews: some View {
        ForEach(children) { child in
            // Type erasure forms the recursion boundary for a runtime-defined tree.
            AnyView(DashboardRenderer(widget: child))
        }
    }
    private var placeholder: String { resolved.availability == .loading ? "Loading…" : "Unavailable" }
    @ViewBuilder private var availabilityNote: some View {
        if resolved.availability == .stale {
            Label("Last known value · stale", systemImage: "clock.badge.exclamationmark")
                .font(.caption).foregroundStyle(.secondary)
        }
    }
    private var status: String { resolved.availability == .ready ? (resolved.value?.string ?? "unknown") : "unknown" }
    private var statusLabel: String {
        if resolved.availability == .stale { return "Stale · last known \(resolved.value?.string ?? "unknown")" }
        return ["clear", "notice", "urgent", "emergency"].contains(status) ? status.capitalized : "Unknown"
    }
    private var statusSymbol: String {
        switch status { case "clear": "checkmark.circle.fill"; case "notice": "exclamationmark.circle.fill"; case "urgent": "exclamationmark.triangle.fill"; case "emergency": "bolt.trianglebadge.exclamationmark.fill"; default: "questionmark.circle" }
    }
    private var statusColor: Color {
        switch status { case "clear": .green; case "notice": .yellow; case "urgent": .red; case "emergency": .primary; default: .secondary }
    }
}
