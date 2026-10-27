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
    private enum Destination: Hashable { case overview, module(String) }
    @EnvironmentObject private var session: SessionStore
    @EnvironmentObject private var windows: DashboardWindows
    @Environment(\.openWindow) private var openWindow
    @State private var selection: Destination? = .overview

    private var modules: [ManagedModule] { session.catalog?.orderedModules ?? [] }
    private var module: ManagedModule? { session.selectedModule }
    private var isOverview: Bool { selection == .overview || module == nil }

    var body: some View {
        NavigationSplitView {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 10) {
                    Image(nsImage: NSApp.applicationIconImage).resizable().frame(width: 36, height: 36).accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 3) {
                        Text("Spiritglass").font(.headline)
                        Text("A window into everything.").font(.caption).foregroundStyle(.secondary)
                    }
                }.padding(.horizontal, 18).padding(.vertical, 24)
                List(selection: $selection) {
                    Label("Overview", systemImage: "square.grid.2x2").tag(Destination.overview)
                    Section("Your modules") {
                        ForEach(modules) { module in
                            HStack(spacing: 9) {
                                Circle().fill(lampColor(module)).frame(width: 7, height: 7)
                                Text(module.name).lineLimit(1)
                                if module.reloading { ProgressView().controlSize(.mini) }
                            }.tag(Destination.module(module.id))
                        }
                    }
                }.listStyle(.sidebar)
                VStack(alignment: .leading, spacing: 6) {
                    Label(session.connected ? "\(modules.count) modules · connected" : "Disconnected", systemImage: session.connected ? "waveform.path" : "circle.dashed")
                        .font(.caption.weight(.medium))
                    Text("Local to this Mac").font(.caption2).foregroundStyle(.secondary)
                }.padding(18)
            }.navigationSplitViewColumnWidth(min: 220, ideal: 260)
        } detail: {
            ScrollView {
                VStack(alignment: .leading, spacing: 26) {
                    HStack(alignment: .top) {
                        VStack(alignment: .leading, spacing: 8) {
                            Text(isOverview ? "Your window, open." : module?.name ?? "Dashboard")
                                .font(.system(size: 30, weight: .semibold, design: .rounded))
                            Text(isOverview ? "A little clarity across your connected systems." : module?.state.module?.description ?? "An independent module, connected to this Mac.")
                                .font(.callout).foregroundStyle(.secondary)
                        }
                        Spacer(minLength: 16)
                        if let module, !isOverview { management(module) }
                    }
                    if !session.connected {
                        banner("Waiting for a connection", text: session.connectionMessage, symbol: "antenna.radiowaves.left.and.right.slash")
                    }
                    if let error = session.actionError {
                        HStack {
                            Label(error, systemImage: "exclamationmark.circle").foregroundStyle(.orange)
                            Spacer()
                            Button("Dismiss") { session.actionError = nil }.buttonStyle(.plain)
                        }.font(.callout).padding().background(.orange.opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
                    }
                    if isOverview { overview }
                    else if let module {
                        moduleDashboard(module)
                    }
                }.padding(36).frame(maxWidth: 1120, alignment: .leading).frame(maxWidth: .infinity)
            }
            .background(Color(nsColor: .windowBackgroundColor))
            .navigationTitle(isOverview ? "Overview" : module?.name ?? "Dashboard")
        }
        .onChange(of: selection) { _, value in
            if case .module(let id) = value { session.selectModule(id) }
            else { session.selectModule(nil) }
        }
        .onChange(of: session.selectedModuleID) { _, value in
            if value == nil && selection != .overview { selection = .overview }
        }
        .onAppear {
            windows.open = {
                if let window = NSApp.windows.first(where: { $0.canBecomeMain && $0.styleMask.contains(.titled) && !$0.isExcludedFromWindowsMenu }) {
                    window.makeKeyAndOrderFront(nil)
                } else { openWindow(id: "dashboard") }
                NSApp.activate(ignoringOtherApps: true)
            }
        }
    }

    private func lampColor(_ module: ManagedModule) -> Color {
        guard module.enabled, module.phase != "dormant" else { return .secondary }
        return StatusLamp.color(AttentionSummary.resolve(snapshot: module.state, connected: session.connected).severity)
    }
    private func phaseLabel(_ module: ManagedModule) -> String {
        guard session.connected else { return "Stale" }
        if module.reloading { return "Validating replacement" }
        switch module.phase {
        case "ready": return "Live"
        case "restarting": return "Recovering"
        case "starting": return "Starting"
        case "disabled": return "Disabled"
        case "dormant": return "On demand"
        default: return "Unavailable"
        }
    }
    @ViewBuilder private func management(_ module: ManagedModule) -> some View {
        VStack(alignment: .trailing, spacing: 8) {
            Text(phaseLabel(module)).font(.caption.weight(.medium)).foregroundStyle(.secondary)
            HStack {
                if module.lifecycle == "on_demand" {
                    Button(module.phase == "dormant" ? "Start" : "Stop") {
                        session.control(module.phase == "dormant" ? "module.activate" : "module.deactivate", moduleID: module.id)
                    }.disabled(!session.connected || !module.enabled || session.busyModules.contains(module.id))
                }
                Button {
                    session.control("module.reload", moduleID: module.id)
                } label: { Label("Reload", systemImage: "arrow.clockwise") }
                    .disabled(!session.connected || !module.enabled || module.reloading || session.busyModules.contains(module.id))
            }.buttonStyle(.bordered)
        }
    }
    @ViewBuilder private func moduleDashboard(_ module: ManagedModule) -> some View {
        if let error = module.reloadError {
            banner("Replacement couldn’t start", text: "\(error)\nYour current version keeps running.", symbol: "arrow.triangle.2.circlepath")
        } else if module.reloading {
            banner("Validating a replacement", text: "Your current version stays live until the new process publishes valid initial data.", symbol: "arrow.triangle.2.circlepath")
        }
        if module.phase == "disabled" {
            banner("Module disabled", text: "Enable it in its module.json manifest to start monitoring.", symbol: "pause.circle")
        } else if module.phase == "dormant" {
            banner("Ready on demand", text: "Start this module when you need it.", symbol: "play.circle")
        } else if let diagnostic = module.state.diagnostic {
            banner("Module is recovering", text: diagnostic, symbol: "exclamationmark.bubble")
        }
        if let view = module.state.view {
            DashboardRenderer(widget: view).id(module.id)
        } else if ["starting", "restarting"].contains(module.phase) {
            ProgressView("Waiting for valid module data…").frame(maxWidth: .infinity, minHeight: 160)
        }
        HStack(spacing: 16) {
            Text("Version \(module.generation)")
            if let pid = module.pid { Text("Process \(pid)") }
            if module.restartCount > 0 { Text("\(module.restartCount) restarts") }
        }.font(.caption).foregroundStyle(.secondary)
        if !module.history.isEmpty {
            DisclosureGroup("Recent diagnostics") {
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(Array(module.history.enumerated()), id: \.offset) { _, diagnostic in
                        HStack(alignment: .top, spacing: 12) {
                            Text(Date(timeIntervalSince1970: Double(diagnostic.atMs) / 1000), style: .time).monospacedDigit()
                            Text(diagnostic.message).textSelection(.enabled)
                        }.font(.caption).foregroundStyle(.secondary)
                    }
                }.frame(maxWidth: .infinity, alignment: .leading).padding(.top, 10)
            }
        }
    }
    private var overview: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack(spacing: 10) {
                Image(nsImage: StatusLamp.image(for: session.attention))
                Text(session.attention.title).font(.title3.weight(.medium))
            }
            if let diagnostics = session.catalog?.diagnostics {
                ForEach(Array(diagnostics.enumerated()), id: \.offset) { _, diagnostic in
                    banner("Module installation needs attention", text: diagnostic.message, symbol: "exclamationmark.folder")
                }
            }
            if modules.isEmpty {
                ContentUnavailableView("A place for your systems", systemImage: "rectangle.connected.to.line.below", description: Text("Install a module folder in ~/.spiritglass/modules. Its dashboard will appear automatically."))
                    .frame(maxWidth: .infinity, minHeight: 220)
            }
            ForEach(modules) { module in
                Button {
                    selection = .module(module.id)
                } label: {
                    HStack(alignment: .top, spacing: 18) {
                        Circle().fill(lampColor(module)).frame(width: 10, height: 10).padding(.top, 6)
                        VStack(alignment: .leading, spacing: 7) {
                            Text(module.name).font(.headline)
                            Text(module.state.module?.description ?? "Installed module").font(.callout).foregroundStyle(.secondary)
                            Text(phaseLabel(module)).font(.caption).foregroundStyle(.secondary)
                            if let error = module.reloadError { Text(error).font(.caption).foregroundStyle(.orange).lineLimit(2) }
                        }
                        Spacer()
                        Image(systemName: "chevron.right").font(.caption).foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading).padding(24)
                    .background(.background, in: RoundedRectangle(cornerRadius: 16))
                }.buttonStyle(.plain)
            }
        }
    }
    private func banner(_ title: String, text: String, symbol: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Label(title, systemImage: symbol).font(.headline)
            Text(text).font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
        }.frame(maxWidth: .infinity, alignment: .leading).padding(20).background(.quaternary, in: RoundedRectangle(cornerRadius: 14))
    }
}
