//! panel.rs 的单元测试 — 覆盖树构建、父级解析和显示排序等纯逻辑。
//!
//! 作者：logic

use super::*;
use chrono::NaiveDateTime;
use warp_ssh_manager::{NodeKind, SshNode};

// --- 测试辅助 --------------------------------------------------------------

fn ts() -> NaiveDateTime {
    chrono::DateTime::from_timestamp(0, 0)
        .unwrap()
        .naive_utc()
}

fn folder(id: &str, parent_id: Option<&str>, name: &str, sort_order: i32) -> SshNode {
    SshNode {
        id: id.to_string(),
        parent_id: parent_id.map(|s| s.to_string()),
        kind: NodeKind::Folder,
        name: name.to_string(),
        sort_order,
        created_at: ts(),
        updated_at: ts(),
        is_collapsed: false,
    }
}

fn server(id: &str, parent_id: Option<&str>, name: &str, sort_order: i32) -> SshNode {
    SshNode {
        id: id.to_string(),
        parent_id: parent_id.map(|s| s.to_string()),
        kind: NodeKind::Server,
        name: name.to_string(),
        sort_order,
        created_at: ts(),
        updated_at: ts(),
        is_collapsed: false,
    }
}

// --- resolve_parent_for_new_node 测试 ---------------------------------------

#[test]
fn parent_no_selection_returns_none() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    assert_eq!(resolve_parent_for_new_node(None, &nodes), None);
}

#[test]
fn parent_folder_selected_returns_folder_id() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    assert_eq!(
        resolve_parent_for_new_node(Some("f1"), &nodes),
        Some("f1".to_string())
    );
}

#[test]
fn parent_server_at_root_selected_returns_none() {
    let nodes = vec![server("s1", None, "srv", 0)];
    assert_eq!(resolve_parent_for_new_node(Some("s1"), &nodes), None);
}

#[test]
fn parent_server_under_folder_selected_returns_folder_id() {
    let nodes = vec![
        folder("f1", None, "Prod", 0),
        server("s1", Some("f1"), "web", 0),
    ];
    assert_eq!(
        resolve_parent_for_new_node(Some("s1"), &nodes),
        Some("f1".to_string())
    );
}

#[test]
fn parent_invalid_selected_id_returns_none() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    assert_eq!(resolve_parent_for_new_node(Some("nonexistent"), &nodes), None);
}

#[test]
fn parent_empty_nodes_with_selection_returns_none() {
    assert_eq!(resolve_parent_for_new_node(Some("any"), &[]), None);
}

#[test]
fn parent_deeply_nested_folder_selected_returns_immediate_parent() {
    // f1(root) → f2(child) → s1(grandchild server)
    let nodes = vec![
        folder("f1", None, "L0", 0),
        folder("f2", Some("f1"), "L1", 0),
        server("s1", Some("f2"), "srv", 0),
    ];
    // 选中 f2 → 新节点创建在 f2 下
    assert_eq!(
        resolve_parent_for_new_node(Some("f2"), &nodes),
        Some("f2".to_string())
    );
    // 选中 s1 → 新节点创建在 s1 的父级(f2)下（兄弟语义）
    assert_eq!(
        resolve_parent_for_new_node(Some("s1"), &nodes),
        Some("f2".to_string())
    );
}

// --- compute_depths 测试 ---------------------------------------------------

#[test]
fn depths_empty_nodes() {
    let depths = compute_depths(&[]);
    assert!(depths.is_empty());
}

#[test]
fn depths_single_root() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    let depths = compute_depths(&nodes);
    assert_eq!(depths["f1"], 0);
}

#[test]
fn depths_nested_tree() {
    let nodes = vec![
        folder("f1", None, "Root", 0),
        folder("f2", Some("f1"), "Child", 0),
        server("s1", Some("f2"), "Grandchild", 0),
    ];
    let depths = compute_depths(&nodes);
    assert_eq!(depths["f1"], 0);
    assert_eq!(depths["f2"], 1);
    assert_eq!(depths["s1"], 2);
}

#[test]
fn depths_multiple_roots() {
    let nodes = vec![
        folder("f1", None, "Root1", 0),
        folder("f2", None, "Root2", 1),
        server("s1", Some("f1"), "srv", 0),
        server("s2", Some("f2"), "srv", 0),
    ];
    let depths = compute_depths(&nodes);
    assert_eq!(depths["f1"], 0);
    assert_eq!(depths["f2"], 0);
    assert_eq!(depths["s1"], 1);
    assert_eq!(depths["s2"], 1);
}

// --- sort_for_display 测试 -------------------------------------------------

#[test]
fn sort_empty() {
    let depths = HashMap::new();
    let sorted = sort_for_display(vec![], &depths);
    assert!(sorted.is_empty());
}

#[test]
fn sort_single_root() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    assert_eq!(sorted.len(), 1);
    assert_eq!(sorted[0].id, "f1");
}

#[test]
fn sort_respects_parent_child_order() {
    let nodes = vec![
        server("s1", Some("f1"), "web", 0),
        folder("f1", None, "Prod", 0),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    // f1 在前，s1 在后
    assert_eq!(sorted[0].id, "f1");
    assert_eq!(sorted[1].id, "s1");
}

#[test]
fn sort_multiple_roots_by_sort_order() {
    let nodes = vec![
        folder("f2", None, "B", 1),
        folder("f1", None, "A", 0),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    assert_eq!(sorted[0].id, "f1");
    assert_eq!(sorted[1].id, "f2");
}

#[test]
fn sort_deeply_nested() {
    let nodes = vec![
        folder("f1", None, "Root", 0),
        server("s2", Some("f2"), "deep", 1),
        folder("f2", Some("f1"), "Child", 0),
        server("s1", Some("f1"), "shallow", 1),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    let ids: Vec<&str> = sorted.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, &["f1", "f2", "s2", "s1"]);
}

#[test]
fn sort_multiple_roots_with_children() {
    let nodes = vec![
        folder("f2", None, "Stage", 1),
        folder("f1", None, "Prod", 0),
        server("s1", Some("f1"), "web", 0),
        server("s2", Some("f2"), "app", 0),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    let ids: Vec<&str> = sorted.iter().map(|n| n.id.as_str()).collect();
    // f1(Prod) 及其子节点在前，f2(Stage) 及其子节点在后
    assert_eq!(ids, &["f1", "s1", "f2", "s2"]);
}
