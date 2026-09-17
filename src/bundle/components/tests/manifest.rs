use crate::bundle::ComponentFiles;
use crate::bundle::components::*;
use crate::component::build_vtree;
use crate::config::RunConfig;
use crate::decompose::SelectionCtx;
use crate::tests::common::{Scratch, chain_components};

/// The split partitions the reduced space, which is what
/// [`manifest_matches_vtree`] answers and no caller outside this module can
/// ask. Two chains plus a variable in no clause: the free variable is the id
/// that belongs to no component and still has to be covered.
#[test]
fn the_manifest_and_the_whole_vtree_cover_the_same_reduced_space() {
    let formula = chain_components(&[5, 1, 5]);
    let cfg = RunConfig {
        vtree_spec: "minfill-primal".to_string(),
        ..Default::default()
    };
    let built = build_vtree(&formula, &cfg, &SelectionCtx::plain()).expect("the vtree must build");
    assert!(built.components.is_some(), "two chains must split");
    let whole = built.vtree.clone();

    let dir = Scratch::new("manifest-covers");
    let ComponentFiles { manifest: m, .. } = write_components(
        dir.path(),
        &formula,
        &built,
        None,
        ComponentWriteOptions::default(),
    )
    .expect("components must write");

    assert!(
        manifest_matches_vtree(&m, &whole),
        "components + free vars must cover the reduced space"
    );
}
