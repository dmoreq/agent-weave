//! Integration tests: mem_touch and mem_write --touch over MCP.

use std::fs;

use serde_json::json;

use super::common::{init_and_list_tools, mem_fixture, result_text, rpc, spawn_with};

#[test]
fn test_mcp_mem_touch_and_mem_write_touch() {
    let repo = mem_fixture("mcp-touch");
    let mut srv = spawn_with(&repo, true); // --all-tools includes mem_touch
    init_and_list_tools(&mut srv);

    // 1. Write note via MCP
    let write_call = rpc(
        &mut srv,
        1,
        "tools/call",
        json!({
            "name": "mem_write",
            "arguments": {
                "key": "mcp-test-touch",
                "body": "note body for mcp touch test",
                "kind": "gotcha",
                "refs": "utils.py"
            }
        }),
    );
    assert_eq!(write_call["result"]["isError"], false);
    assert!(result_text(&write_call).contains("ok: 'mcp-test-touch' rev 1 written"));

    // 2. Touch via mem_touch MCP tool
    let touch_call = rpc(
        &mut srv,
        2,
        "tools/call",
        json!({
            "name": "mem_touch",
            "arguments": {
                "key": "mcp-test-touch"
            }
        }),
    );
    assert_eq!(touch_call["result"]["isError"], false);
    assert!(result_text(&touch_call).contains("ok: 'mcp-test-touch' rev 1 touched"));

    // 3. Touch via mem_write with touch: true
    let write_touch_call = rpc(
        &mut srv,
        3,
        "tools/call",
        json!({
            "name": "mem_write",
            "arguments": {
                "key": "mcp-test-touch",
                "body": "note body for mcp touch test",
                "kind": "gotcha",
                "refs": "utils.py",
                "touch": true
            }
        }),
    );
    let text = result_text(&write_touch_call);
    assert!(
        text.contains("ok: 'mcp-test-touch' touched at rev 1"),
        "actual: {text}"
    );

    // 4. Query with ref filter
    let query_ref_call = rpc(
        &mut srv,
        4,
        "tools/call",
        json!({
            "name": "mem_query",
            "arguments": {
                "ref": "utils.py"
            }
        }),
    );
    assert_eq!(query_ref_call["result"]["isError"], false);
    assert!(result_text(&query_ref_call).contains("mcp-test-touch"));

    // 5. Query with empty string terms
    let query_empty_call = rpc(
        &mut srv,
        5,
        "tools/call",
        json!({
            "name": "mem_query",
            "arguments": {
                "terms": ""
            }
        }),
    );
    assert_eq!(query_empty_call["result"]["isError"], false);
    assert!(result_text(&query_empty_call).contains("mcp-test-touch"));

    let _ = fs::remove_dir_all(&repo);
}
