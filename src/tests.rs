// Copyright (c) 2024-2025 Ihor
// SPDX-License-Identifier: BSL-1.1
// See LICENSE file for details

use serde_json::{json, Value};
use std::path::PathBuf;
use tempfile::TempDir;

use crate::{JsonRpcRequest, JsonRpcResponse, McpServer};

fn make_request(id: i64, method: &str, params: Option<Value>) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(id)),
        method: method.to_string(),
        params,
    }
}

fn assert_success(response: &JsonRpcResponse) {
    assert!(response.error.is_none(), "Expected success but got error: {:?}", response.error);
    assert!(response.result.is_some(), "Expected result but got none");
}

fn assert_error(response: &JsonRpcResponse, expected_code: i32) {
    assert!(response.error.is_some(), "Expected error but got success");
    assert_eq!(response.error.as_ref().unwrap().code, expected_code);
}

async fn create_test_project() -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path().to_path_buf();

    // Create a simple Rust file for testing
    let src_dir = project_path.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    std::fs::write(
        src_dir.join("main.rs"),
        r#"
/// Main entry point
fn main() {
    println!("Hello, world!");
    helper();
}

/// A helper function
fn helper() {
    let x = 42;
    println!("x = {}", x);
}

/// A struct for testing
struct TestStruct {
    field: i32,
}

impl TestStruct {
    fn new(value: i32) -> Self {
        Self { field: value }
    }
}
"#,
    )
    .unwrap();

    // Create .adi directory
    std::fs::create_dir_all(project_path.join(".adi/tree/embeddings")).unwrap();
    std::fs::create_dir_all(project_path.join(".adi/cache")).unwrap();

    (temp_dir, project_path)
}

// ==================== LIFECYCLE TESTS ====================

#[tokio::test]
async fn test_initialize_without_root_uri() {
    let mut server = McpServer::new();
    let request = make_request(1, "initialize", None);
    let response = server.handle_request(request).await;

    assert_success(&response);
    let result = response.result.unwrap();
    assert_eq!(result["protocolVersion"], "2024-11-05");
    assert!(result["capabilities"]["tools"].is_object());
    assert!(result["capabilities"]["resources"].is_object());
    assert!(result["capabilities"]["prompts"].is_object());
    assert_eq!(result["serverInfo"]["name"], "adi-mcp");
}

#[tokio::test]
async fn test_initialize_with_root_uri() {
    let (_temp_dir, project_path) = create_test_project().await;
    let mut server = McpServer::new();

    let request = make_request(
        1,
        "initialize",
        Some(json!({
            "rootUri": format!("file://{}", project_path.display())
        })),
    );
    let response = server.handle_request(request).await;

    assert_success(&response);
    assert_eq!(server.project_path, project_path);
}

#[tokio::test]
async fn test_initialized_method() {
    let mut server = McpServer::new();
    let request = make_request(1, "initialized", None);
    let response = server.handle_request(request).await;

    assert_success(&response);
    assert_eq!(response.result.unwrap(), json!({}));
}

#[tokio::test]
async fn test_ping_method() {
    let mut server = McpServer::new();
    let request = make_request(1, "ping", None);
    let response = server.handle_request(request).await;

    assert_success(&response);
    assert_eq!(response.result.unwrap(), json!({}));
}

#[tokio::test]
async fn test_unknown_method() {
    let mut server = McpServer::new();
    let request = make_request(1, "unknown/method", None);
    let response = server.handle_request(request).await;

    assert_error(&response, -32601); // Method not found
}

// ==================== TOOLS TESTS ====================

#[tokio::test]
async fn test_tools_list() {
    let mut server = McpServer::new();
    let request = make_request(1, "tools/list", None);
    let response = server.handle_request(request).await;

    assert_success(&response);
    let result = response.result.unwrap();
    let tools = result["tools"].as_array().unwrap();

    assert!(!tools.is_empty());

    // Check expected tools exist
    let tool_names: Vec<&str> = tools
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();

    assert!(tool_names.contains(&"search"));
    assert!(tool_names.contains(&"search_symbols"));
    assert!(tool_names.contains(&"search_files"));
    assert!(tool_names.contains(&"get_symbol"));
    assert!(tool_names.contains(&"get_file"));
    assert!(tool_names.contains(&"get_callers"));
    assert!(tool_names.contains(&"get_callees"));
    assert!(tool_names.contains(&"get_symbol_usage"));
    assert!(tool_names.contains(&"get_tree"));
    assert!(tool_names.contains(&"index"));
    assert!(tool_names.contains(&"status"));
}

#[tokio::test]
async fn test_tools_list_schema_format() {
    let mut server = McpServer::new();
    let request = make_request(1, "tools/list", None);
    let response = server.handle_request(request).await;

    assert_success(&response);
    let tools = response.result.unwrap()["tools"].as_array().unwrap().clone();

    for tool in tools {
        assert!(tool["name"].is_string());
        assert!(tool["description"].is_string());
        assert!(tool["inputSchema"].is_object());
        assert_eq!(tool["inputSchema"]["type"], "object");
    }
}

#[tokio::test]
async fn test_tools_call_without_initialization() {
    let mut server = McpServer::new();
    let request = make_request(
        1,
        "tools/call",
        Some(json!({
            "name": "status",
            "arguments": {}
        })),
    );
    let response = server.handle_request(request).await;

    assert_error(&response, -32603); // Server error (ADI not initialized)
}

#[tokio::test]
async fn test_tools_call_missing_params() {
    let mut server = McpServer::new();
    let request = make_request(1, "tools/call", None);
    let response = server.handle_request(request).await;

    assert_error(&response, -32602); // Invalid params
}

#[tokio::test]
async fn test_tools_call_missing_tool_name() {
    let mut server = McpServer::new();
    let request = make_request(1, "tools/call", Some(json!({ "arguments": {} })));
    let response = server.handle_request(request).await;

    assert_error(&response, -32602); // Invalid params
}

#[tokio::test]
async fn test_tools_call_unknown_tool() {
    let (_temp_dir, project_path) = create_test_project().await;
    let mut server = McpServer::new();

    // Initialize first
    let init_request = make_request(
        1,
        "initialize",
        Some(json!({ "rootUri": format!("file://{}", project_path.display()) })),
    );
    server.handle_request(init_request).await;

    let request = make_request(
        2,
        "tools/call",
        Some(json!({
            "name": "unknown_tool",
            "arguments": {}
        })),
    );
    let response = server.handle_request(request).await;

    assert_error(&response, -32602);
}

// ==================== RESOURCES TESTS ====================

#[tokio::test]
async fn test_resources_list_without_initialization() {
    let mut server = McpServer::new();
    let request = make_request(1, "resources/list", None);
    let response = server.handle_request(request).await;

    assert_success(&response);
    let result = response.result.unwrap();
    assert_eq!(result["resources"], json!([]));
}

#[tokio::test]
async fn test_resources_list_with_initialization() {
    let (_temp_dir, project_path) = create_test_project().await;
    let mut server = McpServer::new();

    // Initialize
    let init_request = make_request(
        1,
        "initialize",
        Some(json!({ "rootUri": format!("file://{}", project_path.display()) })),
    );
    server.handle_request(init_request).await;

    let request = make_request(2, "resources/list", None);
    let response = server.handle_request(request).await;

    assert_success(&response);
    let result = response.result.unwrap();
    let resources = result["resources"].as_array().unwrap();

    // Should have at least status, tree, and config
    assert!(resources.len() >= 3);

    let uris: Vec<&str> = resources.iter().map(|r| r["uri"].as_str().unwrap()).collect();
    assert!(uris.contains(&"adi://status"));
    assert!(uris.contains(&"adi://tree"));
    assert!(uris.contains(&"adi://config"));
}

#[tokio::test]
async fn test_resources_read_missing_uri() {
    let (_temp_dir, project_path) = create_test_project().await;
    let mut server = McpServer::new();

    let init_request = make_request(
        1,
        "initialize",
        Some(json!({ "rootUri": format!("file://{}", project_path.display()) })),
    );
    server.handle_request(init_request).await;

    let request = make_request(2, "resources/read", Some(json!({})));
    let response = server.handle_request(request).await;

    assert_error(&response, -32602);
}

#[tokio::test]
async fn test_resources_read_unknown_uri() {
    let (_temp_dir, project_path) = create_test_project().await;
    let mut server = McpServer::new();

    let init_request = make_request(
        1,
        "initialize",
        Some(json!({ "rootUri": format!("file://{}", project_path.display()) })),
    );
    server.handle_request(init_request).await;

    let request = make_request(
        2,
        "resources/read",
        Some(json!({ "uri": "adi://unknown" })),
    );
    let response = server.handle_request(request).await;

    assert_error(&response, -32602);
}

#[tokio::test]
async fn test_resources_read_status() {
    let (_temp_dir, project_path) = create_test_project().await;
    let mut server = McpServer::new();

    let init_request = make_request(
        1,
        "initialize",
        Some(json!({ "rootUri": format!("file://{}", project_path.display()) })),
    );
    server.handle_request(init_request).await;

    let request = make_request(
        2,
        "resources/read",
        Some(json!({ "uri": "adi://status" })),
    );
    let response = server.handle_request(request).await;

    assert_success(&response);
    let result = response.result.unwrap();
    let contents = result["contents"].as_array().unwrap();
    assert_eq!(contents.len(), 1);
    assert_eq!(contents[0]["uri"], "adi://status");
    assert_eq!(contents[0]["mimeType"], "application/json");
    assert!(contents[0]["text"].is_string());
}

#[tokio::test]
async fn test_resources_subscribe() {
    let mut server = McpServer::new();

    let request = make_request(
        1,
        "resources/subscribe",
        Some(json!({ "uri": "adi://status" })),
    );
    let response = server.handle_request(request).await;

    assert_success(&response);
    assert!(server.subscribed_resources.contains("adi://status"));
}

#[tokio::test]
async fn test_resources_unsubscribe() {
    let mut server = McpServer::new();
    server.subscribed_resources.insert("adi://status".to_string());

    let request = make_request(
        1,
        "resources/unsubscribe",
        Some(json!({ "uri": "adi://status" })),
    );
    let response = server.handle_request(request).await;

    assert_success(&response);
    assert!(!server.subscribed_resources.contains("adi://status"));
}

#[tokio::test]
async fn test_resource_templates_list() {
    let mut server = McpServer::new();
    let request = make_request(1, "resources/templates/list", None);
    let response = server.handle_request(request).await;

    assert_success(&response);
    let result = response.result.unwrap();
    let templates = result["resourceTemplates"].as_array().unwrap();

    assert!(!templates.is_empty());

    let uri_templates: Vec<&str> = templates
        .iter()
        .map(|t| t["uriTemplate"].as_str().unwrap())
        .collect();
    assert!(uri_templates.contains(&"adi://file/{path}"));
    assert!(uri_templates.contains(&"adi://symbol/{id}"));
}

// ==================== PROMPTS TESTS ====================

#[tokio::test]
async fn test_prompts_list() {
    let mut server = McpServer::new();
    let request = make_request(1, "prompts/list", None);
    let response = server.handle_request(request).await;

    assert_success(&response);
    let result = response.result.unwrap();
    let prompts = result["prompts"].as_array().unwrap();

    assert!(!prompts.is_empty());

    let prompt_names: Vec<&str> = prompts.iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert!(prompt_names.contains(&"code_review"));
    assert!(prompt_names.contains(&"explain_symbol"));
    assert!(prompt_names.contains(&"find_similar"));
    assert!(prompt_names.contains(&"analyze_dependencies"));
    assert!(prompt_names.contains(&"summarize_file"));
    assert!(prompt_names.contains(&"refactor_suggestions"));
    assert!(prompt_names.contains(&"architecture_overview"));
}

#[tokio::test]
async fn test_prompts_list_schema_format() {
    let mut server = McpServer::new();
    let request = make_request(1, "prompts/list", None);
    let response = server.handle_request(request).await;

    assert_success(&response);
    let prompts = response.result.unwrap()["prompts"].as_array().unwrap().clone();

    for prompt in prompts {
        assert!(prompt["name"].is_string());
        assert!(prompt["description"].is_string());
        // arguments may be empty array or have items
        assert!(prompt["arguments"].is_array());
    }
}

#[tokio::test]
async fn test_prompts_get_missing_name() {
    let (_temp_dir, project_path) = create_test_project().await;
    let mut server = McpServer::new();

    let init_request = make_request(
        1,
        "initialize",
        Some(json!({ "rootUri": format!("file://{}", project_path.display()) })),
    );
    server.handle_request(init_request).await;

    let request = make_request(2, "prompts/get", Some(json!({ "arguments": {} })));
    let response = server.handle_request(request).await;

    assert_error(&response, -32602);
}

#[tokio::test]
async fn test_prompts_get_unknown_prompt() {
    let (_temp_dir, project_path) = create_test_project().await;
    let mut server = McpServer::new();

    let init_request = make_request(
        1,
        "initialize",
        Some(json!({ "rootUri": format!("file://{}", project_path.display()) })),
    );
    server.handle_request(init_request).await;

    let request = make_request(
        2,
        "prompts/get",
        Some(json!({ "name": "unknown_prompt", "arguments": {} })),
    );
    let response = server.handle_request(request).await;

    assert_error(&response, -32602);
}

#[tokio::test]
async fn test_prompts_get_architecture_overview() {
    let (_temp_dir, project_path) = create_test_project().await;
    let mut server = McpServer::new();

    let init_request = make_request(
        1,
        "initialize",
        Some(json!({ "rootUri": format!("file://{}", project_path.display()) })),
    );
    server.handle_request(init_request).await;

    let request = make_request(
        2,
        "prompts/get",
        Some(json!({ "name": "architecture_overview", "arguments": {} })),
    );
    let response = server.handle_request(request).await;

    assert_success(&response);
    let result = response.result.unwrap();
    assert!(result["description"].is_string());
    let messages = result["messages"].as_array().unwrap();
    assert!(!messages.is_empty());
    assert_eq!(messages[0]["role"], "user");
}

#[tokio::test]
async fn test_prompts_get_find_similar() {
    let (_temp_dir, project_path) = create_test_project().await;
    let mut server = McpServer::new();

    let init_request = make_request(
        1,
        "initialize",
        Some(json!({ "rootUri": format!("file://{}", project_path.display()) })),
    );
    server.handle_request(init_request).await;

    let request = make_request(
        2,
        "prompts/get",
        Some(json!({
            "name": "find_similar",
            "arguments": { "description": "error handling" }
        })),
    );
    let response = server.handle_request(request).await;

    assert_success(&response);
    let result = response.result.unwrap();
    let messages = result["messages"].as_array().unwrap();
    let text = messages[0]["content"]["text"].as_str().unwrap();
    assert!(text.contains("error handling"));
}

// ==================== COMPLETION TESTS ====================

#[tokio::test]
async fn test_completion_without_initialization() {
    let mut server = McpServer::new();
    let request = make_request(
        1,
        "completion/complete",
        Some(json!({
            "ref": { "type": "ref/prompt", "name": "code_review" },
            "argument": { "name": "focus", "value": "sec" }
        })),
    );
    let response = server.handle_request(request).await;

    assert_success(&response);
    let result = response.result.unwrap();
    assert_eq!(result["completion"]["values"], json!([]));
    assert_eq!(result["completion"]["hasMore"], false);
}

#[tokio::test]
async fn test_completion_focus_argument() {
    let (_temp_dir, project_path) = create_test_project().await;
    let mut server = McpServer::new();

    let init_request = make_request(
        1,
        "initialize",
        Some(json!({ "rootUri": format!("file://{}", project_path.display()) })),
    );
    server.handle_request(init_request).await;

    let request = make_request(
        2,
        "completion/complete",
        Some(json!({
            "ref": { "type": "ref/prompt", "name": "code_review" },
            "argument": { "name": "focus", "value": "sec" }
        })),
    );
    let response = server.handle_request(request).await;

    assert_success(&response);
    let result = response.result.unwrap();
    let values = result["completion"]["values"].as_array().unwrap();
    assert!(values.contains(&json!("security")));
}

#[tokio::test]
async fn test_completion_direction_argument() {
    let (_temp_dir, project_path) = create_test_project().await;
    let mut server = McpServer::new();

    let init_request = make_request(
        1,
        "initialize",
        Some(json!({ "rootUri": format!("file://{}", project_path.display()) })),
    );
    server.handle_request(init_request).await;

    let request = make_request(
        2,
        "completion/complete",
        Some(json!({
            "ref": { "type": "ref/prompt", "name": "analyze_dependencies" },
            "argument": { "name": "direction", "value": "call" }
        })),
    );
    let response = server.handle_request(request).await;

    assert_success(&response);
    let result = response.result.unwrap();
    let values = result["completion"]["values"].as_array().unwrap();
    assert!(values.contains(&json!("callers")));
    assert!(values.contains(&json!("callees")));
}

#[tokio::test]
async fn test_completion_missing_ref() {
    let mut server = McpServer::new();
    let request = make_request(
        1,
        "completion/complete",
        Some(json!({
            "argument": { "name": "focus", "value": "sec" }
        })),
    );
    let response = server.handle_request(request).await;

    assert_error(&response, -32602);
}

// ==================== JSON-RPC FORMAT TESTS ====================

#[tokio::test]
async fn test_response_has_correct_jsonrpc_version() {
    let mut server = McpServer::new();
    let request = make_request(1, "ping", None);
    let response = server.handle_request(request).await;

    assert_eq!(response.jsonrpc, "2.0");
}

#[tokio::test]
async fn test_response_preserves_request_id() {
    let mut server = McpServer::new();

    // Test with numeric ID
    let request = make_request(42, "ping", None);
    let response = server.handle_request(request).await;
    assert_eq!(response.id, json!(42));

    // Test with string ID
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!("test-id")),
        method: "ping".to_string(),
        params: None,
    };
    let response = server.handle_request(request).await;
    assert_eq!(response.id, json!("test-id"));
}

#[tokio::test]
async fn test_response_null_id_when_missing() {
    let mut server = McpServer::new();
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: None,
        method: "ping".to_string(),
        params: None,
    };
    let response = server.handle_request(request).await;
    assert_eq!(response.id, Value::Null);
}
