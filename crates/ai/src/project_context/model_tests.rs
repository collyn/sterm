use super::*;
use std::path::PathBuf;

#[test]
fn test_find_applicable_rules_empty_rules() {
    let rules = ProjectRules { rules: vec![] };
    let path = PathBuf::from("/a/b/c/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert!(result.is_empty());
}

#[test]
fn test_find_applicable_rules_no_matching_rules() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(Path::new("/x/y/WARP.md"), "content1".to_string());
    rules.upsert_rule(Path::new("/z/AGENTS.md"), "content2".to_string());

    let path = PathBuf::from("/a/b/c/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert!(result.is_empty());
}

#[test]
fn test_find_applicable_rules_single_matching_rule() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(Path::new("/a/WARP.md"), "content1".to_string());
    rules.upsert_rule(Path::new("/x/AGENTS.md"), "content2".to_string());

    let path = PathBuf::from("/a/b/c/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].path, PathBuf::from("/a/WARP.md"));
}

#[test]
fn test_find_applicable_rules_includes_all_ancestor_rules() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(Path::new("/a/WARP.md"), "root_warp".to_string());
    rules.upsert_rule(Path::new("/a/b/WARP.md"), "nested_warp".to_string());
    rules.upsert_rule(Path::new("/a/b/c/WARP.md"), "deep_warp".to_string());

    let path = PathBuf::from("/a/b/c/d/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 3);

    // All should be WARP.md files (same priority), order is not specified by depth
    // Just verify all expected rules are present
    let paths: Vec<PathBuf> = result.iter().map(|r| r.path.clone()).collect();
    assert!(paths.contains(&PathBuf::from("/a/WARP.md")));
    assert!(paths.contains(&PathBuf::from("/a/b/WARP.md")));
    assert!(paths.contains(&PathBuf::from("/a/b/c/WARP.md")));
}

#[test]
fn test_find_applicable_rules_multiple_patterns() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(Path::new("/a/b/AGENTS.md"), "agents_content".to_string());
    rules.upsert_rule(Path::new("/a/WARP.md"), "warp_content".to_string());

    let path = PathBuf::from("/a/b/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 2);

    assert_eq!(result[0].path, PathBuf::from("/a/b/AGENTS.md"));
    assert_eq!(result[0].content, "agents_content");
    assert_eq!(result[1].path, PathBuf::from("/a/WARP.md"));
    assert_eq!(result[1].content, "warp_content");
}

#[test]
fn test_find_applicable_rules_exact_path_match() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(Path::new("/a/b/WARP.md"), "exact_match".to_string());

    let path = PathBuf::from("/a/b/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].path, PathBuf::from("/a/b/WARP.md"));
    assert_eq!(result[0].content, "exact_match");
}

#[test]
fn test_find_applicable_rules_ignores_deeper_paths() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(Path::new("/a/WARP.md"), "applicable".to_string());
    rules.upsert_rule(Path::new("/a/b/c/d/e/WARP.md"), "too_deep".to_string()); // Path doesn't contain /a/b

    let path = PathBuf::from("/a/b/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].path, PathBuf::from("/a/WARP.md"));
    assert_eq!(result[0].content, "applicable");
}

#[test]
fn test_find_applicable_rules_handles_root_path() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(Path::new("/WARP.md"), "root_rule".to_string());

    let path = PathBuf::from("/a/b/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].path, PathBuf::from("/WARP.md"));
    assert_eq!(result[0].content, "root_rule");
}

#[test]
fn test_find_applicable_rules_complex_scenario() {
    // This test covers the example from the original request:
    // For path /a/b/c/file.rs with rules:
    // - /a/WARP.md
    // - /a/AGENTS.md
    // - /a/b/WARP.md
    // - /a/b/AGENTS.md
    let mut rules = ProjectRules::default();

    rules.upsert_rule(Path::new("/a/WARP.md"), "a_warp".to_string());
    rules.upsert_rule(Path::new("/a/AGENTS.md"), "a_agents".to_string());
    rules.upsert_rule(Path::new("/a/b/WARP.md"), "ab_warp".to_string());
    rules.upsert_rule(Path::new("/a/b/AGENTS.md"), "ab_agents".to_string());
    rules.upsert_rule(Path::new("/x/WARP.md"), "irrelevant".to_string()); // Should be ignored

    let path = PathBuf::from("/a/b/c/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 2);

    // Expect only WARP.md files to be included as they have higher priority.
    assert_eq!(result[0].path, PathBuf::from("/a/WARP.md"));
    assert_eq!(result[0].content, "a_warp");
    assert_eq!(result[1].path, PathBuf::from("/a/b/WARP.md"));
    assert_eq!(result[1].content, "ab_warp");
}

#[test]
fn test_find_applicable_rules_handles_unknown_file_patterns() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(Path::new("/a/WARP.md"), "known_pattern".to_string());
    rules.upsert_rule(Path::new("/a/UNKNOWN.md"), "unknown_pattern".to_string());
    let path = PathBuf::from("/a/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 1);

    assert_eq!(result[0].path, PathBuf::from("/a/WARP.md"));
    assert_eq!(result[0].content, "known_pattern");
}

#[test]
fn test_find_applicable_rules_with_relative_paths() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(Path::new("src/WARP.md"), "src_warp".to_string());
    rules.upsert_rule(
        Path::new("src/components/WARP.md"),
        "components_warp".to_string(),
    );

    let path = PathBuf::from("src/components/Button.tsx");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 2);

    // Both are WARP.md files (same priority), order within same priority is not guaranteed
    // Just verify both rules are present
    let paths: Vec<PathBuf> = result.iter().map(|r| r.path.clone()).collect();
    assert!(paths.contains(&PathBuf::from("src/WARP.md")));
    assert!(paths.contains(&PathBuf::from("src/components/WARP.md")));
}

// ---------------------------------------------------------------------------
// Fast-path tests(针对 ProjectContextModel::scan_fast_path + fast_path_entry_still_valid)
// ---------------------------------------------------------------------------
//
// 这些测试走真实 fs(临时目录),不依赖 ModelContext。覆盖:
//   - cwd 本身有 AGENTS.md → 命中
//   - WARP.md 优先于 AGENTS.md(同目录)
//   - 祖先目录规则可被 findUp 到
//   - 无规则 → 返 None
//   - 失效检查:修改文件 mtime → still_valid 返 false
//   - 失效检查:在 walked 目录里新增规则文件 → still_valid 返 false

#[cfg(feature = "local_fs")]
#[test]
fn fast_path_finds_agents_md_in_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().canonicalize().unwrap();
    std::fs::write(cwd.join("AGENTS.md"), "hello agents").unwrap();

    let entry = ProjectContextModel::scan_fast_path(&cwd);
    assert_eq!(entry.rules.len(), 1, "期望命中 1 个规则");
    assert_eq!(entry.rules[0].content, "hello agents");
    assert_eq!(entry.rules[0].path, cwd.join("AGENTS.md"));
    assert_eq!(entry.root_path, cwd);
    assert_eq!(entry.stamps.len(), 1);
}

#[cfg(feature = "local_fs")]
#[test]
fn fast_path_warp_md_takes_priority_over_agents_md() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().canonicalize().unwrap();
    std::fs::write(cwd.join("WARP.md"), "warp wins").unwrap();
    std::fs::write(cwd.join("AGENTS.md"), "agents loses").unwrap();

    let entry = ProjectContextModel::scan_fast_path(&cwd);
    assert_eq!(
        entry.rules.len(),
        1,
        "同目录两个规则文件只取 1 个(对齐 RuleAtPath::respected_rule)"
    );
    assert_eq!(entry.rules[0].content, "warp wins");
    assert_eq!(entry.rules[0].path, cwd.join("WARP.md"));
}

#[cfg(feature = "local_fs")]
#[test]
fn fast_path_finds_rule_in_ancestor_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let sub = root.join("a").join("b").join("c");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(root.join("AGENTS.md"), "ancestor rule").unwrap();

    let entry = ProjectContextModel::scan_fast_path(&sub);
    assert_eq!(entry.rules.len(), 1);
    assert_eq!(entry.rules[0].content, "ancestor rule");
    assert_eq!(entry.root_path, root);
}

#[cfg(feature = "local_fs")]
#[test]
fn fast_path_returns_empty_when_no_rules_anywhere() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().canonicalize().unwrap();

    let entry = ProjectContextModel::scan_fast_path(&cwd);
    assert!(entry.rules.is_empty());
    // root_path 回退为 cwd(语义对齐 find_applicable_rules 的 None 返回)
    assert_eq!(entry.root_path, cwd);
    // walked_dir_stamps 不为空(至少 walked 了 cwd 本身,negative cache 可生效)
    assert!(!entry.walked_dir_stamps.is_empty());
}

#[cfg(feature = "local_fs")]
#[test]
fn fast_path_still_valid_when_nothing_changed() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().canonicalize().unwrap();
    std::fs::write(cwd.join("AGENTS.md"), "stable").unwrap();

    let entry = ProjectContextModel::scan_fast_path(&cwd);
    assert!(ProjectContextModel::fast_path_entry_still_valid(&entry));
}

#[cfg(feature = "local_fs")]
#[test]
fn fast_path_invalidated_when_rule_file_mtime_changes() {
    use filetime::{set_file_mtime, FileTime};

    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().canonicalize().unwrap();
    let rule = cwd.join("AGENTS.md");
    std::fs::write(&rule, "v1").unwrap();

    let entry = ProjectContextModel::scan_fast_path(&cwd);
    assert!(ProjectContextModel::fast_path_entry_still_valid(&entry));

    // 把 mtime 推后 10s → 缓存应被检测为失效
    let stamp = entry.stamps[0].1;
    let new_mtime = FileTime::from_system_time(stamp + std::time::Duration::from_secs(10));
    set_file_mtime(&rule, new_mtime).unwrap();
    assert!(!ProjectContextModel::fast_path_entry_still_valid(&entry));
}

#[cfg(feature = "local_fs")]
#[test]
fn fast_path_invalidated_when_new_rule_file_appears_in_walked_dir() {
    use filetime::{set_file_mtime, FileTime};

    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().canonicalize().unwrap();

    // 首次扫描:未命中任何规则(negative cache)
    let entry = ProjectContextModel::scan_fast_path(&cwd);
    assert!(entry.rules.is_empty());

    // 记录原始目录 mtime,后面手动推进一下以触发失效检测。
    // 只在这里才创建文件 — 但某些文件系统创建文件不会马上更新目录 mtime。
    // 为了测试稳定性,创建文件后显式调 set_file_mtime 保证目录 mtime 不同于 stamp。
    std::fs::write(cwd.join("AGENTS.md"), "new!").unwrap();
    let original_dir_mtime = entry.walked_dir_stamps[0].1;
    let bumped =
        FileTime::from_system_time(original_dir_mtime + std::time::Duration::from_secs(10));
    set_file_mtime(&cwd, bumped).unwrap();

    assert!(!ProjectContextModel::fast_path_entry_still_valid(&entry));
}

#[cfg(feature = "local_fs")]
#[test]
fn fast_path_walk_depth_bounded() {
    // 验证 MAX_WALK_DEPTH 生效:深度超过上限的目录不会 stat 到顶层规则文件。
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    // 构造 ≥7 层子目录(MAX_WALK_DEPTH = 6)
    let mut deep = root.clone();
    for seg in ["a", "b", "c", "d", "e", "f", "g"] {
        deep.push(seg);
    }
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(root.join("AGENTS.md"), "top").unwrap();

    let entry = ProjectContextModel::scan_fast_path(&deep);
    // 走不到顶层,拿不到规则
    assert!(entry.rules.is_empty(), "深度超限后不应 stat 到顶层规则文件");
    // walked_dir_stamps 不超过 MAX_WALK_DEPTH
    assert!(entry.walked_dir_stamps.len() <= 6);
}

// ---------------------------------------------------------------------------
// CLAUDE.md 默认识别专项测试
// ---------------------------------------------------------------------------

#[cfg(feature = "local_fs")]
#[test]
fn fast_path_finds_claude_md() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().canonicalize().unwrap();
    std::fs::write(cwd.join("CLAUDE.md"), "claude rules").unwrap();

    let entry = ProjectContextModel::scan_fast_path(&cwd);
    assert_eq!(entry.rules.len(), 1, "CLAUDE.md 应被默认识别");
    assert_eq!(entry.rules[0].content, "claude rules");
    assert_eq!(entry.rules[0].path, cwd.join("CLAUDE.md"));
}

#[cfg(feature = "local_fs")]
#[test]
fn fast_path_warp_md_priority_over_claude_md() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().canonicalize().unwrap();
    std::fs::write(cwd.join("WARP.md"), "warp wins").unwrap();
    std::fs::write(cwd.join("CLAUDE.md"), "claude loses").unwrap();

    let entry = ProjectContextModel::scan_fast_path(&cwd);
    assert_eq!(entry.rules.len(), 1);
    assert_eq!(entry.rules[0].content, "warp wins");
    assert_eq!(entry.rules[0].path, cwd.join("WARP.md"));
}

#[cfg(feature = "local_fs")]
#[test]
fn fast_path_agents_md_priority_over_claude_md() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().canonicalize().unwrap();
    std::fs::write(cwd.join("AGENTS.md"), "agents wins").unwrap();
    std::fs::write(cwd.join("CLAUDE.md"), "claude loses").unwrap();

    let entry = ProjectContextModel::scan_fast_path(&cwd);
    assert_eq!(entry.rules.len(), 1);
    assert_eq!(entry.rules[0].content, "agents wins");
    assert_eq!(entry.rules[0].path, cwd.join("AGENTS.md"));
}

#[test]
fn upsert_rule_recognizes_claude_md() {
    // 纯内存路径(不走 fs)验证 ProjectRules::upsert_rule 能识别 CLAUDE.md
    let mut rules = ProjectRules::default();
    rules.upsert_rule(Path::new("/a/CLAUDE.md"), "claude in /a".to_string());

    let path = PathBuf::from("/a/sub/file.rs");
    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].path, PathBuf::from("/a/CLAUDE.md"));
    assert_eq!(result[0].content, "claude in /a");
}

#[test]
fn upsert_rule_priority_three_way() {
    // 同目录同时存在 WARP / AGENTS / CLAUDE → 只拿优先级最高的 WARP
    let mut rules = ProjectRules::default();
    rules.upsert_rule(Path::new("/a/WARP.md"), "warp".to_string());
    rules.upsert_rule(Path::new("/a/AGENTS.md"), "agents".to_string());
    rules.upsert_rule(Path::new("/a/CLAUDE.md"), "claude".to_string());

    let result = rules
        .find_active_or_applicable_rules(&PathBuf::from("/a/x.rs"))
        .active_rules;
    assert_eq!(result.len(), 1, "同目录多个规则文件只取优先级最高的");
    assert_eq!(result[0].path, PathBuf::from("/a/WARP.md"));
}

#[test]
fn upsert_rule_priority_agents_beats_claude() {
    // 同目录 AGENTS + CLAUDE → 取 AGENTS
    let mut rules = ProjectRules::default();
    rules.upsert_rule(Path::new("/a/AGENTS.md"), "agents".to_string());
    rules.upsert_rule(Path::new("/a/CLAUDE.md"), "claude".to_string());

    let result = rules
        .find_active_or_applicable_rules(&PathBuf::from("/a/x.rs"))
        .active_rules;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].path, PathBuf::from("/a/AGENTS.md"));
}

#[test]
fn remove_rule_recognizes_claude_md() {
    let mut rules = ProjectRules::default();
    rules.upsert_rule(Path::new("/a/CLAUDE.md"), "x".to_string());
    rules.upsert_rule(Path::new("/a/AGENTS.md"), "y".to_string());

    let removed = rules.remove_rule(Path::new("/a/CLAUDE.md"));
    assert!(removed.is_some(), "能移除 CLAUDE.md");

    // 移除 CLAUDE 后 AGENTS 仍保留为该目录的生效规则
    let result = rules
        .find_active_or_applicable_rules(&PathBuf::from("/a/x.rs"))
        .active_rules;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].path, PathBuf::from("/a/AGENTS.md"));
}

#[test]
fn upsert_rule_case_insensitive_filename() {
    // 大小写不敏感:claude.md / Agents.MD 也能识别
    let mut rules = ProjectRules::default();
    rules.upsert_rule(Path::new("/a/claude.md"), "lower".to_string());

    let result = rules
        .find_active_or_applicable_rules(&PathBuf::from("/a/x.rs"))
        .active_rules;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].path, PathBuf::from("/a/claude.md"));
}
