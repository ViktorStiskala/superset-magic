use super::*;

// ── operations_for: location gating ──────────────────────────────────────

/// Worktree always gets the unified Sync + Pack, regardless of the branch value
/// passed (branch is irrelevant for a worktree).
#[test]
fn worktree_ops_are_sync_and_pack() {
    for branch in [Branch::Init, Branch::Migrate, Branch::Normal] {
        let ops = operations_for(Location::Worktree, branch);
        assert_eq!(
            ops,
            vec![MenuOp::Sync, MenuOp::Pack],
            "worktree branch={branch:?} must offer Sync + Pack"
        );
    }
}

/// Main checkout never offers the worktree-only unified Sync.
#[test]
fn main_checkout_ops_never_include_worktree_ops() {
    for branch in [Branch::Init, Branch::Migrate, Branch::Normal] {
        let ops = operations_for(Location::Main, branch);
        assert!(
            !ops.contains(&MenuOp::Sync),
            "main checkout must not offer Sync; branch={branch:?}"
        );
    }
}

// ── operations_for: main-checkout branch → op mapping ────────────────────

/// Branch::Migrate → exactly [Migrate].
#[test]
fn migrate_branch_offers_migrate_op() {
    let ops = operations_for(Location::Main, Branch::Migrate);
    assert_eq!(ops, vec![MenuOp::Migrate]);
}

/// Branch::Normal → [EditConfig, Pack].
#[test]
fn normal_branch_offers_edit_config_and_pack() {
    let ops = operations_for(Location::Main, Branch::Normal);
    assert_eq!(ops, vec![MenuOp::EditConfig, MenuOp::Pack]);
}

/// Pack is offered where magic.json exists: any worktree, and Main+Normal.
/// It is NOT offered on the un-initialized Init/Migrate branches.
#[test]
fn pack_offered_only_where_magic_json_exists() {
    assert!(operations_for(Location::Worktree, Branch::Normal).contains(&MenuOp::Pack));
    assert!(operations_for(Location::Main, Branch::Normal).contains(&MenuOp::Pack));
    assert!(!operations_for(Location::Main, Branch::Init).contains(&MenuOp::Pack));
    assert!(!operations_for(Location::Main, Branch::Migrate).contains(&MenuOp::Pack));
}

// ── Invariant: every op belongs to exactly one location ──────────────────

/// Location-specific ops must not overlap. `Pack` is intentionally shared
/// across both locations (offered wherever magic.json exists), so it is
/// excluded from the disjointness invariant.
#[test]
fn main_checkout_ops_are_main_only() {
    let main_ops: std::collections::HashSet<MenuOp> =
        [Branch::Migrate, Branch::Init, Branch::Normal]
            .iter()
            .flat_map(|&b| operations_for(Location::Main, b))
            .filter(|op| *op != MenuOp::Pack)
            .collect();
    let worktree_ops: std::collections::HashSet<MenuOp> =
        operations_for(Location::Worktree, Branch::Init)
            .into_iter()
            .filter(|op| *op != MenuOp::Pack)
            .collect();
    // The two sets must be disjoint (ignoring the shared Pack op).
    let overlap: Vec<_> = main_ops.intersection(&worktree_ops).collect();
    assert!(
        overlap.is_empty(),
        "location-specific ops must not overlap; overlap={overlap:?}"
    );
}

// ── Local install rows ───────────────────────────────────────────────────

/// Every branch, the local one included, for the exhaustive checks below.
const ALL_BRANCHES: [Branch; 4] = [Branch::Migrate, Branch::Init, Branch::Normal, Branch::Local];

/// Branch::Init → committed init first, then the local install.
#[test]
fn init_branch_offers_committed_and_local_init() {
    let ops = operations_for(Location::Main, Branch::Init);
    assert_eq!(ops, vec![MenuOp::Init, MenuOp::InitLocal]);
}

/// Branch::Local → edit the local patterns, and pack.
#[test]
fn local_branch_offers_edit_local_config_and_pack() {
    let ops = operations_for(Location::Main, Branch::Local);
    assert_eq!(ops, vec![MenuOp::EditConfigLocal, MenuOp::Pack]);
}

/// The local entries carry the labels the requirements name.
#[test]
fn local_entries_have_their_labels() {
    assert_eq!(MenuOp::InitLocal.to_string(), "Initialize ss-magic locally");
    assert!(
        MenuOp::EditConfigLocal.to_string().contains("magic.local.json"),
        "the local edit entry must name the file it edits"
    );
}

/// The local entries are main-checkout only: a worktree never offers them.
#[test]
fn worktree_never_offers_local_install_ops() {
    for branch in ALL_BRANCHES {
        let ops = operations_for(Location::Worktree, branch);
        assert!(!ops.contains(&MenuOp::InitLocal), "branch={branch:?}");
        assert!(!ops.contains(&MenuOp::EditConfigLocal), "branch={branch:?}");
    }
}

/// Every op the menu can offer has a handler, so no selection can reach the
/// dispatcher's `unreachable!` arm.
#[test]
fn every_offered_op_has_a_handler() {
    for location in [Location::Main, Location::Worktree] {
        for branch in ALL_BRANCHES {
            for op in operations_for(location, branch) {
                assert!(
                    handler_for(location, op).is_some(),
                    "location={location:?} branch={branch:?} op={op:?} has no handler"
                );
            }
        }
    }
}

/// Both local entries open the local install (the interactive entry doubles
/// as the edit entry), never the committed init.
#[test]
fn local_entries_route_to_the_local_install() {
    assert_eq!(handler_for(Location::Main, MenuOp::InitLocal), Some(Handler::LocalInstall));
    assert_eq!(
        handler_for(Location::Main, MenuOp::EditConfigLocal),
        Some(Handler::LocalInstall)
    );
    assert_eq!(handler_for(Location::Main, MenuOp::Init), Some(Handler::CommittedInit));
}
