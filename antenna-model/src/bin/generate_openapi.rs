//! Writes the generated OpenAPI document to the repository root.
//!
//! The committed spec is generated, never hand-edited: this binary is its single
//! writer. Run after any contract-affecting change, then review
//! `git diff` of the output as a contract change and commit it —
//! `tests/openapi_spec.rs` fails until the committed file matches the code.

use antenna_model::api::openapi::ApiDoc;
use utoipa::OpenApi;

const TARGET_FILE: &str = "openapi.yaml";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("antenna-model crate must live one level under the repo root")?;
    let target = repo_root.join(TARGET_FILE);
    let yaml = ApiDoc::openapi().to_yaml()?;
    // Single trailing newline, matching the committed file's normalization.
    std::fs::write(&target, format!("{}\n", yaml.trim_end()))?;
    println!("wrote {}", target.display());
    Ok(())
}
