//! Drift guard (roadmap C7): the committed spec file must equal the document
//! generated from the code.
//!
//! This is the guard that makes the spec trustworthy: any change to a request/
//! response type, handler doc attribute, or `api::openapi::ApiDoc` shows up here
//! until the regenerated file is committed, and the regeneration diff is the
//! contract-review artifact.

use antenna_model::api::openapi::ApiDoc;
use utoipa::OpenApi;

const SPEC_FILE: &str = "openapi.yaml";

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("antenna-model crate lives one level under the repo root")
        .to_path_buf()
}

#[test]
fn committed_spec_matches_the_generated_document() {
    let generated = ApiDoc::openapi()
        .to_yaml()
        .expect("spec serializes to YAML");
    let generated = format!("{}\n", generated.trim_end());
    let path = repo_root().join(SPEC_FILE);
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    assert_eq!(
        committed, generated,
        "{SPEC_FILE} is out of date with the code.\n\
         Regenerate:  cargo run -p antenna-model --bin generate_openapi\n\
         Then review `git diff {SPEC_FILE}` as a CONTRACT CHANGE and commit it."
    );
}
