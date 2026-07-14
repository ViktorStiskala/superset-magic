use super::*;

// ── operations_for: location gating ──────────────────────────────────────

/// Worktree always gets ForwardSync + ReverseSync + Pack, regardless of the
/// branch value passed (branch is irrelevant for a worktree).
#[test]
fn worktree_ops_are_forward_and_reverse_sync_and_pack() {
    for branch in [Branch::Init, Branch::Migrate, Branch::Normal] {
        let ops = operations_for(Location::Worktree, branch);
        assert_eq!(
            ops,
            vec![MenuOp::ForwardSync, MenuOp::ReverseSync, MenuOp::Pack],
            "worktree branch={branch:?} must offer ForwardSync + ReverseSync + Pack"
        );
    }
}

/// Main checkout never offers ForwardSync or ReverseSync.
#[test]
fn main_checkout_ops_never_include_worktree_ops() {
    for branch in [Branch::Init, Branch::Migrate, Branch::Normal] {
        let ops = operations_for(Location::Main, branch);
        assert!(
            !ops.contains(&MenuOp::ForwardSync),
            "main checkout must not offer ForwardSync; branch={branch:?}"
        );
        assert!(
            !ops.contains(&MenuOp::ReverseSync),
            "main checkout must not offer ReverseSync; branch={branch:?}"
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
