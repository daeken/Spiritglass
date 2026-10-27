import AppKit
import Combine
import SpiritglassCore
import SwiftUI

enum StatusLamp {
    static func color(_ severity: AttentionSeverity?) -> Color {
        switch severity {
        case .clear: .green
        case .notice: .yellow
        case .urgent: .red
        case .emergency: .primary
        case nil: .secondary
        }
    }

    /// A non-template image keeps the traffic-light colors in the system menu bar.
    /// Emergency has a white rim and exclamation mark so black stays visible on dark bars.
    static func image(for summary: AttentionSummary) -> NSImage {
        let image = NSImage(size: NSSize(width: 22, height: 18), flipped: false) { _ in
            let lamp = NSBezierPath(ovalIn: NSRect(x: 2, y: 2, width: 14, height: 14))
            let fill: NSColor = switch summary.severity {
            case .clear: .systemGreen
            case .notice: .systemYellow
            case .urgent: .systemRed
            case .emergency: .black
            case nil: .tertiaryLabelColor
            }
            fill.setFill()
            lamp.fill()
            NSColor.white.withAlphaComponent(0.9).setStroke()
            lamp.lineWidth = 1
            lamp.stroke()
            if summary.severity == .emergency || summary.severity == nil {
                let glyph = summary.severity == .emergency ? "!" : "?"
                (glyph as NSString).draw(at: NSPoint(x: 5.9, y: 2.5), withAttributes: [
                    .font: NSFont.systemFont(ofSize: 11, weight: .heavy),
                    .foregroundColor: summary.severity == .emergency ? NSColor.white : NSColor.labelColor,
                ])
            }
            if summary.hasUnknown && summary.severity != nil {
                let badge = NSBezierPath(ovalIn: NSRect(x: 12, y: 9, width: 9, height: 9))
                NSColor.windowBackgroundColor.setFill(); badge.fill()
                NSColor.labelColor.setStroke(); badge.lineWidth = 0.7; badge.stroke()
                ("?" as NSString).draw(at: NSPoint(x: 14.3, y: 9.2), withAttributes: [
                    .font: NSFont.systemFont(ofSize: 8, weight: .bold),
                    .foregroundColor: NSColor.labelColor,
                ])
            }
            return true
        }
        image.isTemplate = false
        image.accessibilityDescription = "Spiritglass: \(summary.title)"
        return image
    }
}

struct AttentionPopover: View {
    @EnvironmentObject private var session: SessionStore
    let openDashboard: () -> Void

    private var summary: AttentionSummary { session.attention }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack(spacing: 12) {
                Image(nsImage: NSApp.applicationIconImage).resizable().frame(width: 42, height: 42)
                VStack(alignment: .leading, spacing: 3) {
                    Text("Spiritglass").font(.headline)
                    Text("A window into everything.").font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Image(nsImage: StatusLamp.image(for: summary)).accessibilityHidden(true)
            }

            VStack(alignment: .leading, spacing: 6) {
                Text(summary.title).font(.title3.weight(.semibold))
                Text(explanation).font(.callout).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            if !summary.items.isEmpty {
                Divider()
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        ForEach(summary.items) { item in
                            HStack(alignment: .top, spacing: 10) {
                                Image(systemName: symbol(for: item))
                                    .foregroundStyle(item.availability == .ready ? StatusLamp.color(item.severity) : .secondary)
                                    .frame(width: 18)
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(item.label).font(.callout.weight(.medium))
                                    Text(item.detail).font(.caption).foregroundStyle(.secondary)
                                }
                                Spacer()
                            }.accessibilityElement(children: .combine)
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 220)
                .fixedSize(horizontal: false, vertical: true)
            }

            if let diagnostic = session.snapshot?.diagnostic, session.connected {
                Text(diagnostic).font(.caption).foregroundStyle(.secondary).lineLimit(4)
            }

            Divider()
            HStack {
                Button("Open Spiritglass") {
                    openDashboard()
                }.buttonStyle(.borderedProminent).tint(.teal)
                Spacer()
                Button("Quit") { NSApp.terminate(nil) }.buttonStyle(.borderless)
            }
        }
        .padding(22)
        .frame(width: 340)
    }

    private var explanation: String {
        if !session.connected { return "Supervisor disconnected. Last known values are marked stale." }
        if session.snapshot?.health == "starting" { return "Connecting to your module’s live status…" }
        if session.snapshot?.health != "ready" { return "Your module is unavailable. Last known values are marked stale." }
        if summary.items.isEmpty { return "This dashboard hasn’t declared a status indicator." }
        if summary.hasUnknown { return "Some monitoring is unknown or stale. Open the dashboard for details." }
        return session.snapshot?.module?.name ?? "Live module status"
    }

    private func symbol(for item: AttentionItem) -> String {
        if item.availability == .stale { return "clock.badge.exclamationmark" }
        if item.availability != .ready { return "questionmark.circle" }
        switch item.severity {
        case .clear: return "checkmark.circle.fill"
        case .notice: return "exclamationmark.circle.fill"
        case .urgent: return "exclamationmark.triangle.fill"
        case .emergency: return "bolt.trianglebadge.exclamationmark.fill"
        case nil: return "questionmark.circle"
        }
    }
}

/// An explicit status item also supports keyboard/menu access to the same popover.
@MainActor
final class StatusBarController: NSObject {
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
    private let popover = NSPopover()
    private var subscription: AnyCancellable?

    init(session: SessionStore, windows: DashboardWindows) {
        super.init()
        guard let button = item.button else { return }
        button.target = self
        button.action = #selector(toggle)
        popover.behavior = .transient
        let content = AttentionPopover(openDashboard: { [weak self] in
            self?.popover.performClose(nil)
            windows.open()
        }).environmentObject(session)
        let host = NSHostingController(rootView: content)
        host.sizingOptions = .preferredContentSize
        popover.contentViewController = host
        popover.contentSize = NSSize(width: 340, height: 290)
        update(session.attention)
        subscription = session.$snapshot.combineLatest(session.$connected)
            .map { AttentionSummary.resolve(snapshot: $0, connected: $1) }
            .removeDuplicates()
            .receive(on: RunLoop.main)
            .sink { [weak self] summary in self?.update(summary) }
    }

    private func update(_ summary: AttentionSummary) {
        item.button?.image = StatusLamp.image(for: summary)
        item.button?.toolTip = "Spiritglass: \(summary.title)"
        item.button?.setAccessibilityLabel("Spiritglass: \(summary.title)")
    }

    @objc private func toggle() {
        if popover.isShown { popover.performClose(nil) } else { show() }
    }

    func show() {
        guard let button = item.button else { return }
        NSApp.activate(ignoringOtherApps: true)
        popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
        popover.contentViewController?.view.window?.makeKey()
    }
}
