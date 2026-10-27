fn main() -> std::io::Result<()> {
    let schema = schemars::schema_for!(spiritglass_protocol::ProtocolCatalog);
    let destination =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../protocol/v1.schema.json");
    std::fs::write(
        destination,
        format!("{}\n", serde_json::to_string_pretty(&schema)?),
    )?;
    let session = schemars::schema_for!(spiritglass_protocol::SessionProtocolCatalog);
    let destination =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../protocol/v2.schema.json");
    std::fs::write(
        destination,
        format!("{}\n", serde_json::to_string_pretty(&session)?),
    )
}
