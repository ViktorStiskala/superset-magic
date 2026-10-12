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

/// Branch::Init → exactly [Init].
#[test]
fn init_branch_offers_init_op() {
    let ops = operations_for(Location::Main, Branch::Init);
    assert_eq!(ops, vec![MenuOp::Init]);
}

/// Branch::Normal → [EditConfig, Pack, RefreshOrca].
#[test]
fn normal_branch_offers_edit_config_pack_and_refresh_orca() {
    let ops = operations_for(Location::Main, Branch::Normal);
    assert_eq!(
        ops,
        vec![MenuOp::EditConfig, MenuOp::Pack, MenuOp::RefreshOrca]
    );
}

/// The Orca refresh needs an initialized `magic.json` in the main checkout:
/// it is offered there on the Normal branch and nowhere else. Init and migrate
/// write the Orca files themselves, and the worktree list is unchanged.
#[test]
fn refresh_orca_is_offered_only_on_the_main_checkouts_normal_branch() {
    assert!(operations_for(Location::Main, Branch::Normal).contains(&MenuOp::RefreshOrca));
    assert!(!operations_for(Location::Main, Branch::Init).contains(&MenuOp::RefreshOrca));
    assert!(!operations_for(Location::Main, Branch::Migrate).contains(&MenuOp::RefreshOrca));
    for branch in [Branch::Init, Branch::Migrate, Branch::Normal] {
        assert!(!operations_for(Location::Worktree, branch).contains(&MenuOp::RefreshOrca));
    }
}

#[test]
fn every_menu_entry_has_its_own_label() {
    let labels: std::collections::HashSet<String> = [
        MenuOp::Migrate,
        MenuOp::Init,
        MenuOp::EditConfig,
        MenuOp::Sync,
        MenuOp::Pack,
        MenuOp::RefreshOrca,
    ]
    .iter()
    .map(|op| op.to_string())
    .collect();
    assert_eq!(labels.len(), 6);
    assert!(MenuOp::RefreshOrca.to_string().contains("Orca"));
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
