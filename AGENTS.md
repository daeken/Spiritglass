# Working on Spiritglass

Read README.md and docs/architecture.md before changing component boundaries.
The initial specification is framing; docs/roadmap.md records implemented scope.

- Keep the native app generic. Module-specific names, source fields, or business
  logic belong in the module, never the SwiftUI renderer or supervisor.
- Treat stdout as protocol-only in modules. Send logs to stderr.
- Change wire types in spiritglass-protocol, regenerate protocol/v1.schema.json,
  and update docs/protocol.md together. Update the shared snapshot fixture when
  relevant and run both Rust and Swift compatibility tests.
- Use stable source, widget, and action IDs. Unknown/unavailable/stale state must
  remain visibly distinct from a healthy live value.
- Do not add a TCP server, executable UI expressions, or WebViews for the MVP.
- Use scripts/check.sh for the complete verification path. For native UI changes,
  build the app and inspect the affected view in the running application as well.
- Preserve the old process until a replacement has been validated when implementing
  hot replacement. A failed build must never evict a working module.
- Keep module builds independent; adding a module must not require editing the
  app or daemon. Follow the next milestone instead of building every widget upfront.
