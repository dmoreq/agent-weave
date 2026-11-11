//! Dashboard monitor tests.

#[test]
fn monitor_tui_render_all_tabs_coverage() {
    use engram::dashboard::monitor::render::render_screen;
    use engram::dashboard::monitor::state::{MonitorTab, TuiState};

    let temp_dir = std::env::temp_dir().join(format!("test_tui_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    // A monitor row is a live repository: the journaled root must exist.
    let repo = temp_dir.to_string_lossy().into_owned();
    let log_path = temp_dir.join("usage.log");
    std::fs::write(
        &log_path,
        format!(
            r#"{{"ts":1794123456,"tool":"callers","repo":"{repo}","repo_name":"test-repo","agent":"test-agent","latency_us":1200,"exit_code":0,"shown":3,"result_count":3,"args":{{"symbol":"test_fn"}}}}
{{"ts":1794123460,"tool":"search","repo":"{repo}","repo_name":"test-repo","agent":"test-agent","latency_us":3400,"exit_code":0,"args":{{"terms":"cache"}}}}
{{"ts":1794123470,"tool":"mem_get","repo":"{repo}","repo_name":"test-repo","agent":"test-agent","latency_us":900,"exit_code":0,"args":{{"key":"arch:test"}}}}
"#
        ),
    )
    .unwrap();

    let mut state = TuiState::new(None, log_path.clone(), None, false);
    assert_eq!(state.total_repos, 1);
    assert_eq!(state.recent_events.len(), 3);

    // Also test with real global log if it exists!
    let global_log =
        std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".engram/usage.log");
    if global_log.is_file() {
        let mut real_state = TuiState::new(None, global_log, None, false);
        real_state.refresh(None);
        for tab in [
            MonitorTab::Graph,
            MonitorTab::Reuse,
            MonitorTab::Memory,
            MonitorTab::Stream,
        ] {
            real_state.active_tab = tab;
            let mut buf = Vec::new();
            render_screen(&mut buf, &real_state, 80, 24).unwrap();
        }
    }

    for tab in [
        MonitorTab::Graph,
        MonitorTab::Reuse,
        MonitorTab::Memory,
        MonitorTab::Stream,
    ] {
        state.active_tab = tab;
        let mut buf = Vec::new();
        render_screen(&mut buf, &state, 80, 24).unwrap();
        let rendered = String::from_utf8_lossy(&buf);
        assert!(
            rendered.contains(&format!("eg monitor v{}", env!("CARGO_PKG_VERSION"))),
            "Header missing in tab {tab:?}"
        );
        assert!(rendered.contains("Keys:"), "Footer missing in tab {tab:?}");
        match tab {
            MonitorTab::Graph => {
                assert!(rendered.contains("[GRAPH]"));
                assert!(rendered.contains("callers"));
                // Canonical payload keys: real p50 and success-only savings.
                assert!(
                    rendered.contains("1.2ms"),
                    "real p50 must render: {rendered}"
                );
                assert!(rendered.contains("1150"), "savings must render: {rendered}");
            }
            MonitorTab::Reuse => {
                assert!(rendered.contains("[ANTI-REINVENTION]"));
            }
            MonitorTab::Memory => {
                assert!(rendered.contains("[MEMORY]"));
            }
            MonitorTab::Stream => {
                assert!(rendered.contains("[GRAPH ]"));
                assert!(rendered.contains("[REUSE ]"));
                assert!(rendered.contains("[MEMORY]"));
            }
        }
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn monitor_drops_dead_repo_entries_from_state() {
    use engram::dashboard::monitor::state::TuiState;

    let temp_dir = std::env::temp_dir().join(format!("test_tui_dead_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let log_path = temp_dir.join("usage.log");
    // A deleted ordinary repo and a gone scratch repo: nothing live remains.
    let gone = temp_dir.join("eg-reuse-bench-gone");
    let dead = r#"{"tool":"symbols","exit_code":0,"repo":"/nonexistent-eg-tui/archived","repo_name":"archived-repo"}"#;
    let scratch = format!(
        r#"{{"tool":"build","exit_code":0,"repo":"{}","repo_name":"eg-reuse-bench-gone"}}"#,
        gone.display()
    );
    std::fs::write(&log_path, format!("{dead}\n{scratch}\n")).unwrap();

    let state = TuiState::new(None, log_path.clone(), None, false);
    assert_eq!(state.total_repos, 0);
    assert!(state.recent_events.is_empty());
    let _ = std::fs::remove_dir_all(&temp_dir);
}
