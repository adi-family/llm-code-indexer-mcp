use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

struct McpServer {
    adi: Option<adi_core::Adi>,
    project_path: PathBuf,
}

impl McpServer {
    fn new() -> Self {
        Self {
            adi: None,
            project_path: PathBuf::from("."),
        }
    }

    async fn handle_request(&mut self, request: JsonRpcRequest) -> JsonRpcResponse {
        let id = request.id.clone().unwrap_or(Value::Null);

        let result = match request.method.as_str() {
            "initialize" => self.handle_initialize(request.params).await,
            "tools/list" => self.handle_tools_list().await,
            "tools/call" => self.handle_tools_call(request.params).await,
            "resources/list" => self.handle_resources_list().await,
            "resources/read" => self.handle_resources_read(request.params).await,
            "prompts/list" => self.handle_prompts_list().await,
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

        // Initialize ADI
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
                "tools": {},
                "resources": {},
                "prompts": {}
            },
            "serverInfo": {
                "name": "adi-mcp",
                "version": env!("CARGO_PKG_VERSION")
            }
        }))
    }

    async fn handle_tools_list(&self) -> Result<Value, JsonRpcError> {
        Ok(json!({
            "tools": [
                {
                    "name": "search",
                    "description": "Search for code symbols using natural language",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": {
                                "type": "string",
                                "description": "Natural language search query"
                            },
                            "limit": {
                                "type": "integer",
                                "description": "Maximum number of results",
                                "default": 10
                            }
                        },
                        "required": ["query"]
                    }
                },
                {
                    "name": "search_symbols",
                    "description": "Search for symbols by name",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": {
                                "type": "string",
                                "description": "Symbol name to search for"
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
                    "description": "Search for files",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": {
                                "type": "string",
                                "description": "File path or name to search for"
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
                    "description": "Get details of a specific symbol by ID",
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
                    "name": "get_file",
                    "description": "Get file information and its symbols",
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
                    "name": "index",
                    "description": "Index or re-index the project",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "status",
                    "description": "Get indexing status",
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
            message: "ADI not initialized".to_string(),
            data: None,
        })?;

        match name {
            "search" => {
                let query = arguments
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let limit = arguments
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10) as usize;

                let results = adi.search(query, limit).await.map_err(|e| JsonRpcError {
                    code: -32603,
                    message: e.to_string(),
                    data: None,
                })?;

                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&results).unwrap_or_default()
                    }]
                }))
            }
            "search_symbols" => {
                let query = arguments
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let limit = arguments
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10) as usize;

                let results = adi
                    .search_symbols(query, limit)
                    .await
                    .map_err(|e| JsonRpcError {
                        code: -32603,
                        message: e.to_string(),
                        data: None,
                    })?;

                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&results).unwrap_or_default()
                    }]
                }))
            }
            "search_files" => {
                let query = arguments
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let limit = arguments
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10) as usize;

                let results = adi
                    .search_files(query, limit)
                    .await
                    .map_err(|e| JsonRpcError {
                        code: -32603,
                        message: e.to_string(),
                        data: None,
                    })?;

                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&results).unwrap_or_default()
                    }]
                }))
            }
            "get_symbol" => {
                let id = arguments
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| JsonRpcError {
                        code: -32602,
                        message: "Missing symbol id".to_string(),
                        data: None,
                    })?;

                let symbol = adi
                    .get_symbol(adi_core::SymbolId(id))
                    .map_err(|e| JsonRpcError {
                        code: -32603,
                        message: e.to_string(),
                        data: None,
                    })?;

                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&symbol).unwrap_or_default()
                    }]
                }))
            }
            "get_file" => {
                let path = arguments
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| JsonRpcError {
                        code: -32602,
                        message: "Missing file path".to_string(),
                        data: None,
                    })?;

                let file_info = adi
                    .get_file(std::path::Path::new(path))
                    .map_err(|e| JsonRpcError {
                        code: -32603,
                        message: e.to_string(),
                        data: None,
                    })?;

                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&file_info).unwrap_or_default()
                    }]
                }))
            }
            "index" => {
                let progress = adi.index().await.map_err(|e| JsonRpcError {
                    code: -32603,
                    message: e.to_string(),
                    data: None,
                })?;

                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!(
                            "Indexed {} files with {} symbols",
                            progress.files_processed,
                            progress.symbols_indexed
                        )
                    }]
                }))
            }
            "status" => {
                let status = adi.status().map_err(|e| JsonRpcError {
                    code: -32603,
                    message: e.to_string(),
                    data: None,
                })?;

                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&status).unwrap_or_default()
                    }]
                }))
            }
            _ => Err(JsonRpcError {
                code: -32602,
                message: format!("Unknown tool: {}", name),
                data: None,
            }),
        }
    }

    async fn handle_resources_list(&self) -> Result<Value, JsonRpcError> {
        Ok(json!({
            "resources": []
        }))
    }

    async fn handle_resources_read(&self, _params: Option<Value>) -> Result<Value, JsonRpcError> {
        Err(JsonRpcError {
            code: -32602,
            message: "Resource not found".to_string(),
            data: None,
        })
    }

    async fn handle_prompts_list(&self) -> Result<Value, JsonRpcError> {
        Ok(json!({
            "prompts": []
        }))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Setup logging to stderr (stdout is used for JSON-RPC)
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(filter)
        .init();

    info!("Starting ADI MCP server");

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
