import AppKit
import SpiritglassCore
import SwiftUI

@main
struct SpiritglassApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @StateObject private var session: SessionStore
    @StateObject private var windows = DashboardWindows()

    init() {
        let store = SessionStore()
        _session = StateObject(wrappedValue: store)
        store.start()
    }

    var body: some Scene {
        WindowGroup("Spiritglass", id: "dashboard") {
            DashboardWindow().environmentObject(session).environmentObject(windows)
                .frame(minWidth: 760, minHeight: 600)
                .task { session.start() }
                .onAppear { delegate.configure(session: session, windows: windows) }
        }
        .defaultSize(width: 1060, height: 760)
        .windowStyle(.automatic)
        .commands {
            CommandGroup(replacing: .newItem) {}
            CommandGroup(after: .appInfo) {
                Divider()
                Button("Show Status") { delegate.showStatus() }
                    .keyboardShortcut("s", modifiers: [.command, .shift])
                Button("Open Dashboard") { windows.open() }
                    .keyboardShortcut("0", modifiers: .command)
            }
        }
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var statusBar: StatusBarController?
    private var windows: DashboardWindows?

    func configure(session: SessionStore, windows: DashboardWindows) {
        self.windows = windows
        if statusBar == nil { statusBar = StatusBarController(session: session, windows: windows) }
    }
    func showStatus() {
        // Finish menu tracking before presenting a transient popover.
        DispatchQueue.main.async { [weak self] in self?.statusBar?.show() }
    }
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
        if let url = Bundle.main.url(forResource: "Spiritglass", withExtension: "icns"),
           let icon = NSImage(contentsOf: url) {
            // Notify the running Dock as well as declaring the file in Info.plist.
            NSApp.applicationIconImage = icon
        }
        NSApp.activate(ignoringOtherApps: true)
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag { windows?.open() }
        return true
    }
}

@MainActor
final class DashboardWindows: ObservableObject {
    var open: () -> Void = {}
}

struct DashboardWindow: View {
    @EnvironmentObject private var session: SessionStore
    @EnvironmentObject private var windows: DashboardWindows
    @Environment(\.openWindow) private var openWindow
    @State private var selection: String? = "module"

    var body: some View {
        NavigationSplitView {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 10) {
                    Image(nsImage: NSApp.applicationIconImage).resizable().frame(width: 36, height: 36)
                        .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 3) {
                        Text("Spiritglass").font(.headline)
                        Text("A window into everything.").font(.caption).foregroundStyle(.secondary)
                    }
                }.padding(.horizontal, 18).padding(.vertical, 24)
                List(selection: $selection) {
                    Label("Overview", systemImage: "square.grid.2x2").tag("overview")
                    Section("Your modules") {
                        Label(session.snapshot?.module?.name ?? "Example module", systemImage: "square.stack.3d.up").tag("module")
                    }
                }.listStyle(.sidebar)
                VStack(alignment: .leading, spacing: 6) {
                    Label(connectionTitle, systemImage: connectionIcon).font(.caption.weight(.medium))
                    Text("Local to this Mac").font(.caption2).foregroundStyle(.secondary)
                }.padding(18)
            }.navigationSplitViewColumnWidth(min: 220, ideal: 250)
        } detail: {
            ScrollView {
                VStack(alignment: .leading, spacing: 28) {
                    HStack(alignment: .top) {
                        VStack(alignment: .leading, spacing: 8) {
                            Text(selection == "overview" ? "Your window, open." : session.snapshot?.module?.name ?? "Hello, Spiritglass")
                                .font(.system(size: 30, weight: .semibold, design: .rounded))
                            Text(selection == "overview" ? "A little clarity across your connected systems." : session.snapshot?.module?.description ?? "Your modules will appear here when the supervisor connects.")
                                .font(.callout).foregroundStyle(.secondary)
                        }
                        Spacer(minLength: 16)
                        Label(connectionTitle, systemImage: connectionIcon)
                            .font(.caption.weight(.medium)).padding(.horizontal, 12).padding(.vertical, 8)
                            .background(.primary.opacity(0.05), in: Capsule())
                    }
                    if !session.connected || session.snapshot?.health == "unavailable" { diagnosticBanner }
                    if let error = session.actionError {
                        HStack {
                            Label(error, systemImage: "exclamationmark.circle").foregroundStyle(.orange)
                            Spacer()
                            Button("Dismiss") { session.actionError = nil }.buttonStyle(.plain)
                        }.font(.callout).padding().background(.orange.opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
                    }
                    if selection == "overview" {
                        overview
                    } else if let view = session.snapshot?.view {
                        DashboardRenderer(widget: view)
                    } else if session.connected && session.snapshot?.health == "starting" {
                        ProgressView("Starting your module…").frame(maxWidth: .infinity, minHeight: 260)
                    } else {
                        ContentUnavailableView("A place for your systems", systemImage: "rectangle.connected.to.line.below", description: Text("Start the demo supervisor to bring this window to life."))
                            .frame(maxWidth: .infinity, minHeight: 260)
                    }
                    Spacer(minLength: 0)
                }.padding(36).frame(maxWidth: 1060, alignment: .leading).frame(maxWidth: .infinity)
            }
            .background(Color(nsColor: .windowBackgroundColor))
            .navigationTitle(selection == "overview" ? "Overview" : "Dashboard")
        }
        .onAppear {
            windows.open = {
                if let window = NSApp.windows.first(where: {
                    $0.canBecomeMain && $0.styleMask.contains(.titled) && !$0.isExcludedFromWindowsMenu
                }) {
                    window.makeKeyAndOrderFront(nil)
                } else {
                    openWindow(id: "dashboard")
                }
                NSApp.activate(ignoringOtherApps: true)
            }
        }
    }
    private var connectionTitle: String {
        if !session.connected { return "Disconnected" }
        switch session.snapshot?.health { case "ready": return "Live"; case "unavailable": return "Module unavailable"; default: return "Starting" }
    }
    private var connectionIcon: String { session.connected && session.snapshot?.health == "ready" ? "waveform.path" : "circle.dashed" }
    private var diagnosticBanner: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label(session.snapshot?.health == "unavailable" && session.connected ? "Your module needs attention" : "Waiting for a connection", systemImage: "exclamationmark.bubble")
                .font(.headline)
            Text(session.connected ? session.snapshot?.diagnostic ?? "The module is unavailable." : session.connectionMessage).font(.callout).textSelection(.enabled)
            Text(session.connected ? "Restart the supervisor to relaunch the module." : "Run scripts/run-demo.sh from the project folder. Spiritglass reconnects automatically.")
                .font(.caption).foregroundStyle(.secondary)
        }.frame(maxWidth: .infinity, alignment: .leading).padding(20).background(.quaternary, in: RoundedRectangle(cornerRadius: 14))
    }
    private var overview: some View {
        VStack(alignment: .leading, spacing: 18) {
            Label("Your first connection", systemImage: "sparkle").font(.headline)
            Text(session.snapshot?.module?.name ?? "Example module").font(.title2.weight(.medium))
            Text(session.snapshot?.module?.description ?? "An independent module with a native, live dashboard.").foregroundStyle(.secondary)
            Text("Monitoring status: \(connectionTitle)").font(.callout)
            Button("Open dashboard") { selection = "module" }.buttonStyle(.borderedProminent).tint(.teal)
        }.frame(maxWidth: .infinity, alignment: .leading).padding(28).background(.background, in: RoundedRectangle(cornerRadius: 18))
    }
}
