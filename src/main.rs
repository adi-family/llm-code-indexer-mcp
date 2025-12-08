// Copyright (c) 2024-2025 Ihor
// SPDX-License-Identifier: BSL-1.1
// See LICENSE file for details

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[cfg(test)]
mod tests;

// JSON-RPC types
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(Clone))]
pub struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(Clone))]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(Clone))]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// MCP-specific types
#[derive(Debug, Clone, Serialize, Deserialize)]
struct McpResource {
    uri: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct McpResourceContent {
    uri: String,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    blob: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct McpMessage {
    role: String,
    content: McpContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum McpContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { data: String, #[serde(rename = "mimeType")] mime_type: String },
    #[serde(rename = "resource")]
    Resource { resource: McpResourceContent },
}

pub struct McpServer {
    adi: Option<adi_core::Adi>,
    pub project_path: PathBuf,
    pub subscribed_resources: HashSet<String>,
}

impl McpServer {
    pub fn new() -> Self {
        Self {
            adi: None,
            project_path: PathBuf::from("."),
            subscribed_resources: HashSet::new(),
        }
    }

    pub async fn handle_request(&mut self, request: JsonRpcRequest) -> JsonRpcResponse {
        let id = request.id.clone().unwrap_or(Value::Null);

        let result = match request.method.as_str() {
            // Lifecycle
            "initialize" => self.handle_initialize(request.params).await,
            "initialized" => Ok(json!({})),
            "ping" => Ok(json!({})),

            // Tools
            "tools/list" => self.handle_tools_list().await,
            "tools/call" => self.handle_tools_call(request.params).await,

            // Resources
            "resources/list" => self.handle_resources_list(request.params).await,
            "resources/read" => self.handle_resources_read(request.params).await,
            "resources/subscribe" => self.handle_resources_subscribe(request.params).await,
            "resources/unsubscribe" => self.handle_resources_unsubscribe(request.params).await,
            "resources/templates/list" => self.handle_resource_templates_list().await,

            // Prompts
            "prompts/list" => self.handle_prompts_list(request.params).await,
            "prompts/get" => self.handle_prompts_get(request.params).await,

            // Completion (for argument autocompletion)
            "completion/complete" => self.handle_completion(request.params).await,

            _ => Err(JsonRpcError {
                code: -32601,
                message: format!("Method not found: {}", request.method),
                data: None,
            }),
        };

        match result {
            Ok(result) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: Some(result),
                error: None,
            },
            Err(error) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: None,
                error: Some(error),
            },
        }
    }

    async fn handle_initialize(&mut self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        if let Some(params) = params {
            if let Some(root_uri) = params.get("rootUri").and_then(|v| v.as_str()) {
                let path = root_uri.strip_prefix("file://").unwrap_or(root_uri);
                self.project_path = PathBuf::from(path);
            }
        }

        match adi_core::Adi::open(&self.project_path).await {
            Ok(adi) => {
                self.adi = Some(adi);
                info!("ADI initialized for {}", self.project_path.display());
            }
            Err(e) => {
                error!("Failed to initialize ADI: {}", e);
            }
        }

        Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {
                    "listChanged": false
                },
                "resources": {
                    "subscribe": true,
                    "listChanged": true
                },
                "prompts": {
                    "listChanged": false
                }
            },
            "serverInfo": {
                "name": "adi-mcp",
                "version": env!("CARGO_PKG_VERSION")
            }
        }))
    }

    // ==================== TOOLS ====================

    async fn handle_tools_list(&self) -> Result<Value, JsonRpcError> {
        Ok(json!({
            "tools": [
                {
                    "name": "search",
                    "description": "Semantic search for code symbols using natural language. Returns symbols ranked by relevance.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": {
                                "type": "string",
                                "description": "Natural language search query (e.g., 'function that handles user authentication')"
                            },
                            "limit": {
                                "type": "integer",
                                "description": "Maximum number of results (1-100)",
                                "default": 10,
                                "minimum": 1,
                                "maximum": 100
                            }
                        },
                        "required": ["query"]
                    }
                },
                {
                    "name": "search_symbols",
                    "description": "Full-text search for symbols by name. Use for finding specific functions, classes, or variables.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": {
                                "type": "string",
                                "description": "Symbol name to search (supports partial matching)"
                            },
                            "limit": {
                                "type": "integer",
                                "default": 10
                            }
                        },
                        "required": ["query"]
                    }
                },
                {
                    "name": "search_files",
                    "description": "Full-text search for files by path or name.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": {
                                "type": "string",
                                "description": "File path or name pattern"
                            },
                            "limit": {
                                "type": "integer",
                                "default": 10
                            }
                        },
                        "required": ["query"]
                    }
                },
                {
                    "name": "get_symbol",
                    "description": "Get detailed information about a specific symbol by its ID.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "integer",
                                "description": "Symbol ID (from search results)"
                            }
                        },
                        "required": ["id"]
                    }
                },
                {
                    "name": "get_file",
                    "description": "Get file information including all symbols defined in it.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "File path relative to project root"
                            }
                        },
                        "required": ["path"]
                    }
                },
                {
                    "name": "get_callers",
                    "description": "Find all symbols that call/reference a given symbol.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "integer",
                                "description": "Symbol ID to find callers for"
                            }
                        },
                        "required": ["id"]
                    }
                },
                {
                    "name": "get_callees",
                    "description": "Find all symbols that a given symbol calls/references.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "integer",
                                "description": "Symbol ID to find callees for"
                            }
                        },
                        "required": ["id"]
                    }
                },
                {
                    "name": "get_symbol_usage",
                    "description": "Get complete usage statistics for a symbol including reference count, callers, and callees.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "integer",
                                "description": "Symbol ID"
                            }
                        },
                        "required": ["id"]
                    }
                },
                {
                    "name": "get_tree",
                    "description": "Get the complete project structure as a hierarchical tree of files and symbols.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "index",
                    "description": "Index or re-index the project. Parses all source files and generates embeddings.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "status",
                    "description": "Get current indexing status including file/symbol counts and storage size.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                }
            ]
        }))
    }

    async fn handle_tools_call(&mut self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing params".to_string(),
            data: None,
        })?;

        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing tool name".to_string(),
                data: None,
            })?;

        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

        let adi = self.adi.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "ADI not initialized. Call initialize first.".to_string(),
            data: None,
        })?;

        match name {
            "search" => {
                let query = arguments.get("query").and_then(|v| v.as_str()).unwrap_or("");
                let limit = arguments.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                let limit = limit.clamp(1, 100);

                let results = adi.search(query, limit).await.map_err(to_rpc_error)?;
                Ok(tool_result(&serde_json::to_string_pretty(&results).unwrap_or_default()))
            }
            "search_symbols" => {
                let query = arguments.get("query").and_then(|v| v.as_str()).unwrap_or("");
                let limit = arguments.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

                let results = adi.search_symbols(query, limit).await.map_err(to_rpc_error)?;
                Ok(tool_result(&serde_json::to_string_pretty(&results).unwrap_or_default()))
            }
            "search_files" => {
                let query = arguments.get("query").and_then(|v| v.as_str()).unwrap_or("");
                let limit = arguments.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

                let results = adi.search_files(query, limit).await.map_err(to_rpc_error)?;
                Ok(tool_result(&serde_json::to_string_pretty(&results).unwrap_or_default()))
            }
            "get_symbol" => {
                let id = arguments.get("id").and_then(|v| v.as_i64()).ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing symbol id".to_string(),
                    data: None,
                })?;

                let symbol = adi.get_symbol(adi_core::SymbolId(id)).map_err(to_rpc_error)?;
                Ok(tool_result(&serde_json::to_string_pretty(&symbol).unwrap_or_default()))
            }
            "get_file" => {
                let path = arguments.get("path").and_then(|v| v.as_str()).ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing file path".to_string(),
                    data: None,
                })?;

                let file_info = adi.get_file(std::path::Path::new(path)).map_err(to_rpc_error)?;
                Ok(tool_result(&serde_json::to_string_pretty(&file_info).unwrap_or_default()))
            }
            "get_callers" => {
                let id = arguments.get("id").and_then(|v| v.as_i64()).ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing symbol id".to_string(),
                    data: None,
                })?;

                let callers = adi.get_callers(adi_core::SymbolId(id)).map_err(to_rpc_error)?;
                Ok(tool_result(&serde_json::to_string_pretty(&callers).unwrap_or_default()))
            }
            "get_callees" => {
                let id = arguments.get("id").and_then(|v| v.as_i64()).ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing symbol id".to_string(),
                    data: None,
                })?;

                let callees = adi.get_callees(adi_core::SymbolId(id)).map_err(to_rpc_error)?;
                Ok(tool_result(&serde_json::to_string_pretty(&callees).unwrap_or_default()))
            }
            "get_symbol_usage" => {
                let id = arguments.get("id").and_then(|v| v.as_i64()).ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing symbol id".to_string(),
                    data: None,
                })?;

                let usage = adi.get_symbol_usage(adi_core::SymbolId(id)).map_err(to_rpc_error)?;
                Ok(tool_result(&serde_json::to_string_pretty(&usage).unwrap_or_default()))
            }
            "get_tree" => {
                let tree = adi.get_tree().map_err(to_rpc_error)?;
                Ok(tool_result(&serde_json::to_string_pretty(&tree).unwrap_or_default()))
            }
            "index" => {
                let progress = adi.index().await.map_err(to_rpc_error)?;
                Ok(tool_result(&format!(
                    "Indexed {} files with {} symbols. Errors: {}",
                    progress.files_processed,
                    progress.symbols_indexed,
                    if progress.errors.is_empty() { "none".to_string() } else { progress.errors.join(", ") }
                )))
            }
            "status" => {
                let status = adi.status().map_err(to_rpc_error)?;
                Ok(tool_result(&serde_json::to_string_pretty(&status).unwrap_or_default()))
            }
            _ => Err(JsonRpcError {
                code: -32602,
                message: format!("Unknown tool: {}", name),
                data: None,
            }),
        }
    }

    // ==================== RESOURCES ====================

    async fn handle_resources_list(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let adi = match &self.adi {
            Some(adi) => adi,
            None => return Ok(json!({ "resources": [] })),
        };

        let _cursor = params.as_ref().and_then(|p| p.get("cursor")).and_then(|c| c.as_str());

        let mut resources = Vec::new();

        // Add project status resource
        resources.push(McpResource {
            uri: "adi://status".to_string(),
            name: "Index Status".to_string(),
            description: Some("Current indexing status and statistics".to_string()),
            mime_type: Some("application/json".to_string()),
        });

        // Add project tree resource
        resources.push(McpResource {
            uri: "adi://tree".to_string(),
            name: "Project Tree".to_string(),
            description: Some("Hierarchical view of all indexed files and symbols".to_string()),
            mime_type: Some("application/json".to_string()),
        });

        // Add config resource
        resources.push(McpResource {
            uri: "adi://config".to_string(),
            name: "Configuration".to_string(),
            description: Some("Current ADI configuration".to_string()),
            mime_type: Some("application/json".to_string()),
        });

        // Add indexed files as resources
        if let Ok(tree) = adi.get_tree() {
            for file_node in tree.files.iter().take(100) {
                let path_str = file_node.path.to_string_lossy();
                resources.push(McpResource {
                    uri: format!("adi://file/{}", path_str),
                    name: file_node.path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| path_str.to_string()),
                    description: Some(format!("{} file with {} symbols",
                        file_node.language.as_str(),
                        file_node.symbols.len()
                    )),
                    mime_type: Some(language_to_mime(&file_node.language)),
                });
            }
        }

        Ok(json!({
            "resources": resources
        }))
    }

    async fn handle_resources_read(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing params".to_string(),
            data: None,
        })?;

        let uri = params.get("uri").and_then(|v| v.as_str()).ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing uri parameter".to_string(),
            data: None,
        })?;

        let adi = self.adi.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "ADI not initialized".to_string(),
            data: None,
        })?;

        let content = match uri {
            "adi://status" => {
                let status = adi.status().map_err(to_rpc_error)?;
                McpResourceContent {
                    uri: uri.to_string(),
                    mime_type: Some("application/json".to_string()),
                    text: Some(serde_json::to_string_pretty(&status).unwrap_or_default()),
                    blob: None,
                }
            }
            "adi://tree" => {
                let tree = adi.get_tree().map_err(to_rpc_error)?;
                McpResourceContent {
                    uri: uri.to_string(),
                    mime_type: Some("application/json".to_string()),
                    text: Some(serde_json::to_string_pretty(&tree).unwrap_or_default()),
                    blob: None,
                }
            }
            "adi://config" => {
                let config = adi.config();
                McpResourceContent {
                    uri: uri.to_string(),
                    mime_type: Some("application/json".to_string()),
                    text: Some(serde_json::to_string_pretty(&config).unwrap_or_default()),
                    blob: None,
                }
            }
            _ if uri.starts_with("adi://file/") => {
                let path = uri.strip_prefix("adi://file/").unwrap();
                let file_info = adi.get_file(std::path::Path::new(path)).map_err(to_rpc_error)?;

                // Read actual file content
                let full_path = adi.project_path().join(path);
                let file_content = std::fs::read_to_string(&full_path).ok();

                let content_text = if let Some(content) = file_content {
                    json!({
                        "file": file_info.file,
                        "symbols": file_info.symbols,
                        "content": content
                    }).to_string()
                } else {
                    serde_json::to_string_pretty(&file_info).unwrap_or_default()
                };

                McpResourceContent {
                    uri: uri.to_string(),
                    mime_type: Some(language_to_mime(&file_info.file.language)),
                    text: Some(content_text),
                    blob: None,
                }
            }
            _ if uri.starts_with("adi://symbol/") => {
                let id_str = uri.strip_prefix("adi://symbol/").unwrap();
                let id: i64 = id_str.parse().map_err(|_| JsonRpcError {
                    code: -32602,
                    message: "Invalid symbol ID".to_string(),
                    data: None,
                })?;

                let symbol = adi.get_symbol(adi_core::SymbolId(id)).map_err(to_rpc_error)?;
                let usage = adi.get_symbol_usage(adi_core::SymbolId(id)).ok();

                let content_obj = json!({
                    "symbol": symbol,
                    "usage": usage
                });

                McpResourceContent {
                    uri: uri.to_string(),
                    mime_type: Some("application/json".to_string()),
                    text: Some(serde_json::to_string_pretty(&content_obj).unwrap_or_default()),
                    blob: None,
                }
            }
            _ => {
                return Err(JsonRpcError {
                    code: -32602,
                    message: format!("Unknown resource URI: {}", uri),
                    data: None,
                });
            }
        };

        Ok(json!({
            "contents": [content]
        }))
    }

    async fn handle_resources_subscribe(&mut self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing params".to_string(),
            data: None,
        })?;

        let uri = params.get("uri").and_then(|v| v.as_str()).ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing uri parameter".to_string(),
            data: None,
        })?;

        self.subscribed_resources.insert(uri.to_string());
        info!("Subscribed to resource: {}", uri);

        Ok(json!({}))
    }

    async fn handle_resources_unsubscribe(&mut self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing params".to_string(),
            data: None,
        })?;

        let uri = params.get("uri").and_then(|v| v.as_str()).ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing uri parameter".to_string(),
            data: None,
        })?;

        self.subscribed_resources.remove(uri);
        info!("Unsubscribed from resource: {}", uri);

        Ok(json!({}))
    }

    async fn handle_resource_templates_list(&self) -> Result<Value, JsonRpcError> {
        Ok(json!({
            "resourceTemplates": [
                {
                    "uriTemplate": "adi://file/{path}",
                    "name": "Source File",
                    "description": "Access indexed source file with symbols and content",
                    "mimeType": "application/json"
                },
                {
                    "uriTemplate": "adi://symbol/{id}",
                    "name": "Symbol Details",
                    "description": "Get detailed information about a symbol by ID",
                    "mimeType": "application/json"
                }
            ]
        }))
    }

    // ==================== PROMPTS ====================

    async fn handle_prompts_list(&self, _params: Option<Value>) -> Result<Value, JsonRpcError> {
        Ok(json!({
            "prompts": [
                {
                    "name": "code_review",
                    "description": "Review code in a file for quality, bugs, and improvements",
                    "arguments": [
                        {
                            "name": "file_path",
                            "description": "Path to the file to review (relative to project root)",
                            "required": true
                        },
                        {
                            "name": "focus",
                            "description": "Specific aspect to focus on (security, performance, style, bugs)",
                            "required": false
                        }
                    ]
                },
                {
                    "name": "explain_symbol",
                    "description": "Explain what a symbol does and how it's used in the codebase",
                    "arguments": [
                        {
                            "name": "symbol_name",
                            "description": "Name of the symbol to explain",
                            "required": true
                        }
                    ]
                },
                {
                    "name": "find_similar",
                    "description": "Find similar code patterns or implementations in the codebase",
                    "arguments": [
                        {
                            "name": "description",
                            "description": "Description of the code pattern to find",
                            "required": true
                        }
                    ]
                },
                {
                    "name": "analyze_dependencies",
                    "description": "Analyze the dependency graph of a symbol or file",
                    "arguments": [
                        {
                            "name": "target",
                            "description": "Symbol name or file path to analyze",
                            "required": true
                        },
                        {
                            "name": "direction",
                            "description": "Direction to analyze: 'callers' (who uses this), 'callees' (what this uses), or 'both'",
                            "required": false
                        }
                    ]
                },
                {
                    "name": "summarize_file",
                    "description": "Generate a summary of a file's purpose and contents",
                    "arguments": [
                        {
                            "name": "file_path",
                            "description": "Path to the file to summarize",
                            "required": true
                        }
                    ]
                },
                {
                    "name": "refactor_suggestions",
                    "description": "Suggest refactoring opportunities for a symbol or file",
                    "arguments": [
                        {
                            "name": "target",
                            "description": "Symbol name or file path to analyze",
                            "required": true
                        }
                    ]
                },
                {
                    "name": "architecture_overview",
                    "description": "Generate an overview of the project architecture based on indexed symbols",
                    "arguments": []
                }
            ]
        }))
    }

    async fn handle_prompts_get(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing params".to_string(),
            data: None,
        })?;

        let name = params.get("name").and_then(|v| v.as_str()).ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing prompt name".to_string(),
            data: None,
        })?;

        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

        let adi = self.adi.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "ADI not initialized".to_string(),
            data: None,
        })?;

        let messages = match name {
            "code_review" => {
                let file_path = arguments.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
                let focus = arguments.get("focus").and_then(|v| v.as_str()).unwrap_or("general");

                let file_info = adi.get_file(std::path::Path::new(file_path)).ok();
                let full_path = adi.project_path().join(file_path);
                let content = std::fs::read_to_string(&full_path).ok();

                let context = if let Some(info) = &file_info {
                    format!("File: {}\nLanguage: {}\nSymbols: {}\n",
                        file_path,
                        info.file.language.as_str(),
                        info.symbols.iter().map(|s| format!("{} ({})", s.name, s.kind.as_str())).collect::<Vec<_>>().join(", ")
                    )
                } else {
                    format!("File: {}", file_path)
                };

                vec![McpMessage {
                    role: "user".to_string(),
                    content: McpContent::Text {
                        text: format!(
                            "Please review the following code with a focus on {}.\n\n{}\n\nCode:\n```\n{}\n```\n\nProvide specific, actionable feedback.",
                            focus,
                            context,
                            content.unwrap_or_else(|| "[File content not available]".to_string())
                        ),
                    },
                }]
            }
            "explain_symbol" => {
                let symbol_name = arguments.get("symbol_name").and_then(|v| v.as_str()).unwrap_or("");

                let symbols = adi.find_symbols_by_name(symbol_name).unwrap_or_default();
                let usage_info: Vec<_> = symbols.iter().take(3).filter_map(|s| {
                    adi.get_symbol_usage(s.id).ok().map(|u| (s, u))
                }).collect();

                let context = if !usage_info.is_empty() {
                    usage_info.iter().map(|(s, u)| {
                        format!(
                            "Symbol: {} ({})\nFile: {}\nSignature: {}\nDoc: {}\nCallers: {}\nCallees: {}",
                            s.name,
                            s.kind.as_str(),
                            s.file_path.display(),
                            s.signature.as_deref().unwrap_or("N/A"),
                            s.doc_comment.as_deref().unwrap_or("N/A"),
                            u.callers.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", "),
                            u.callees.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")
                        )
                    }).collect::<Vec<_>>().join("\n\n---\n\n")
                } else {
                    format!("No symbol found with name: {}", symbol_name)
                };

                vec![McpMessage {
                    role: "user".to_string(),
                    content: McpContent::Text {
                        text: format!(
                            "Please explain what '{}' does and how it's used in this codebase.\n\nContext from code index:\n{}",
                            symbol_name,
                            context
                        ),
                    },
                }]
            }
            "find_similar" => {
                let description = arguments.get("description").and_then(|v| v.as_str()).unwrap_or("");

                vec![McpMessage {
                    role: "user".to_string(),
                    content: McpContent::Text {
                        text: format!(
                            "Find code in this codebase that is similar to or implements: {}\n\nUse the 'search' tool with semantic search to find relevant symbols, then analyze them.",
                            description
                        ),
                    },
                }]
            }
            "analyze_dependencies" => {
                let target = arguments.get("target").and_then(|v| v.as_str()).unwrap_or("");
                let direction = arguments.get("direction").and_then(|v| v.as_str()).unwrap_or("both");

                let symbols = adi.find_symbols_by_name(target).unwrap_or_default();
                let dep_info: String = symbols.iter().take(1).filter_map(|s| {
                    let callers = if direction != "callees" { adi.get_callers(s.id).ok() } else { None };
                    let callees = if direction != "callers" { adi.get_callees(s.id).ok() } else { None };
                    Some(format!(
                        "Symbol: {} ({})\nFile: {}\nCallers ({}):\n{}\n\nCallees ({}):\n{}",
                        s.name,
                        s.kind.as_str(),
                        s.file_path.display(),
                        callers.as_ref().map_or(0, |c| c.len()),
                        callers.map_or_else(|| "N/A".to_string(), |c| c.iter().map(|x| format!("  - {} ({})", x.name, x.file_path.display())).collect::<Vec<_>>().join("\n")),
                        callees.as_ref().map_or(0, |c| c.len()),
                        callees.map_or_else(|| "N/A".to_string(), |c| c.iter().map(|x| format!("  - {} ({})", x.name, x.file_path.display())).collect::<Vec<_>>().join("\n"))
                    ))
                }).collect();

                vec![McpMessage {
                    role: "user".to_string(),
                    content: McpContent::Text {
                        text: format!(
                            "Analyze the dependency graph for '{}' (direction: {}).\n\nDependency Information:\n{}",
                            target,
                            direction,
                            if dep_info.is_empty() { "No symbol found".to_string() } else { dep_info }
                        ),
                    },
                }]
            }
            "summarize_file" => {
                let file_path = arguments.get("file_path").and_then(|v| v.as_str()).unwrap_or("");

                let file_info = adi.get_file(std::path::Path::new(file_path)).ok();
                let full_path = adi.project_path().join(file_path);
                let content = std::fs::read_to_string(&full_path).ok();

                let symbols_summary = file_info.as_ref().map(|info| {
                    info.symbols.iter().map(|s| {
                        format!("- {} ({}): {}",
                            s.name,
                            s.kind.as_str(),
                            s.doc_comment.as_deref().unwrap_or("no documentation")
                        )
                    }).collect::<Vec<_>>().join("\n")
                }).unwrap_or_default();

                vec![McpMessage {
                    role: "user".to_string(),
                    content: McpContent::Text {
                        text: format!(
                            "Please summarize the purpose and contents of this file.\n\nFile: {}\nLanguage: {}\n\nSymbols:\n{}\n\nCode:\n```\n{}\n```",
                            file_path,
                            file_info.as_ref().map_or("unknown", |i| i.file.language.as_str()),
                            symbols_summary,
                            content.unwrap_or_else(|| "[Content not available]".to_string())
                        ),
                    },
                }]
            }
            "refactor_suggestions" => {
                let target = arguments.get("target").and_then(|v| v.as_str()).unwrap_or("");

                let symbols = adi.find_symbols_by_name(target).unwrap_or_default();
                let context: String = symbols.iter().take(1).filter_map(|s| {
                    let usage = adi.get_symbol_usage(s.id).ok()?;
                    Some(format!(
                        "Symbol: {} ({})\nFile: {}\nReferences: {}\nCallers: {}\nCallees: {}",
                        s.name,
                        s.kind.as_str(),
                        s.file_path.display(),
                        usage.reference_count,
                        usage.callers.len(),
                        usage.callees.len()
                    ))
                }).collect();

                vec![McpMessage {
                    role: "user".to_string(),
                    content: McpContent::Text {
                        text: format!(
                            "Suggest refactoring opportunities for '{}'.\n\nContext:\n{}",
                            target,
                            if context.is_empty() { "No symbol found. Try searching with the 'search' tool.".to_string() } else { context }
                        ),
                    },
                }]
            }
            "architecture_overview" => {
                let tree = adi.get_tree().ok();
                let status = adi.status().ok();

                let overview = if let Some(tree) = tree {
                    let by_language: std::collections::HashMap<String, Vec<_>> = tree.files.iter()
                        .fold(std::collections::HashMap::new(), |mut acc, f| {
                            acc.entry(f.language.as_str().to_string()).or_default().push(f);
                            acc
                        });

                    format!(
                        "Project Statistics:\n- Total files: {}\n- Total symbols: {}\n\nFiles by language:\n{}",
                        status.as_ref().map_or(0, |s| s.indexed_files),
                        status.as_ref().map_or(0, |s| s.indexed_symbols),
                        by_language.iter().map(|(lang, files)| {
                            format!("- {}: {} files", lang, files.len())
                        }).collect::<Vec<_>>().join("\n")
                    )
                } else {
                    "No index available. Run the 'index' tool first.".to_string()
                };

                vec![McpMessage {
                    role: "user".to_string(),
                    content: McpContent::Text {
                        text: format!(
                            "Generate an architecture overview for this project based on the indexed structure.\n\n{}",
                            overview
                        ),
                    },
                }]
            }
            _ => {
                return Err(JsonRpcError {
                    code: -32602,
                    message: format!("Unknown prompt: {}", name),
                    data: None,
                });
            }
        };

        Ok(json!({
            "description": get_prompt_description(name),
            "messages": messages
        }))
    }

    // ==================== COMPLETION ====================

    async fn handle_completion(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing params".to_string(),
            data: None,
        })?;

        let ref_obj = params.get("ref").ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing ref parameter".to_string(),
            data: None,
        })?;

        let ref_type = ref_obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let argument_name = params.get("argument").and_then(|a| a.get("name")).and_then(|n| n.as_str()).unwrap_or("");
        let argument_value = params.get("argument").and_then(|a| a.get("value")).and_then(|v| v.as_str()).unwrap_or("");

        let adi = match &self.adi {
            Some(adi) => adi,
            None => return Ok(json!({ "completion": { "values": [], "hasMore": false } })),
        };

        let completions: Vec<String> = match (ref_type, argument_name) {
            ("ref/prompt", "file_path") | ("ref/resource", _) => {
                if let Ok(tree) = adi.get_tree() {
                    tree.files.iter()
                        .map(|f| f.path.to_string_lossy().to_string())
                        .filter(|p| p.contains(argument_value))
                        .take(20)
                        .collect()
                } else {
                    vec![]
                }
            }
            ("ref/prompt", "symbol_name") | ("ref/prompt", "target") => {
                if !argument_value.is_empty() {
                    adi.search_symbols(argument_value, 20).await
                        .map(|symbols| symbols.into_iter().map(|s| s.name).collect())
                        .unwrap_or_default()
                } else {
                    vec![]
                }
            }
            ("ref/prompt", "focus") => {
                vec!["security", "performance", "style", "bugs", "general"]
                    .into_iter()
                    .filter(|f| f.contains(argument_value))
                    .map(String::from)
                    .collect()
            }
            ("ref/prompt", "direction") => {
                vec!["callers", "callees", "both"]
                    .into_iter()
                    .filter(|d| d.contains(argument_value))
                    .map(String::from)
                    .collect()
            }
            _ => vec![],
        };

        Ok(json!({
            "completion": {
                "values": completions,
                "hasMore": false
            }
        }))
    }
}

// Helper functions

fn to_rpc_error(e: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32603,
        message: e.to_string(),
        data: None,
    }
}

fn tool_result(text: &str) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": text
        }]
    })
}

fn language_to_mime(lang: &adi_core::Language) -> String {
    match lang {
        adi_core::Language::Rust => "text/x-rust",
        adi_core::Language::Python => "text/x-python",
        adi_core::Language::JavaScript => "text/javascript",
        adi_core::Language::TypeScript => "text/typescript",
        adi_core::Language::Java => "text/x-java",
        adi_core::Language::Go => "text/x-go",
        adi_core::Language::C => "text/x-c",
        adi_core::Language::Cpp => "text/x-c++",
        adi_core::Language::CSharp => "text/x-csharp",
        adi_core::Language::Ruby => "text/x-ruby",
        adi_core::Language::Php => "text/x-php",
        adi_core::Language::Kotlin => "text/x-kotlin",
        adi_core::Language::Scala => "text/x-scala",
        adi_core::Language::Swift => "text/x-swift",
        adi_core::Language::Bash => "text/x-shellscript",
        adi_core::Language::Json => "application/json",
        adi_core::Language::Yaml => "text/yaml",
        adi_core::Language::Toml => "text/x-toml",
        adi_core::Language::Xml => "application/xml",
        adi_core::Language::Html => "text/html",
        adi_core::Language::Css => "text/css",
        adi_core::Language::Markdown => "text/markdown",
        _ => "text/plain",
    }.to_string()
}

fn get_prompt_description(name: &str) -> String {
    match name {
        "code_review" => "Code review with focus on quality, bugs, and improvements",
        "explain_symbol" => "Explanation of symbol purpose and usage",
        "find_similar" => "Search for similar code patterns",
        "analyze_dependencies" => "Dependency graph analysis",
        "summarize_file" => "File purpose and content summary",
        "refactor_suggestions" => "Refactoring recommendations",
        "architecture_overview" => "Project architecture overview",
        _ => "Prompt",
    }.to_string()
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(filter)
        .init();

    info!("Starting ADI MCP server v{}", env!("CARGO_PKG_VERSION"));

    let server = Arc::new(Mutex::new(McpServer::new()));

    let stdin = std::io::stdin();
    let reader = BufReader::new(stdin.lock());
    let mut stdout = std::io::stdout();

    for line in reader.lines() {
        let line = line?;

        if line.trim().is_empty() {
            continue;
        }

        debug!("Received: {}", line);

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                let response = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: Value::Null,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {}", e),
                        data: None,
                    }),
                };
                let json = serde_json::to_string(&response)?;
                writeln!(stdout, "{}", json)?;
                stdout.flush()?;
                continue;
            }
        };

        let mut server = server.lock().await;
        let response = server.handle_request(request).await;

        let json = serde_json::to_string(&response)?;
        debug!("Sending: {}", json);
        writeln!(stdout, "{}", json)?;
        stdout.flush()?;
    }

    Ok(())
}
