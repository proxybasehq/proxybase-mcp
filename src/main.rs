/// ProxyBase MCP Server
///
/// A Model Context Protocol (MCP) server that lets AI agents purchase and
/// manage SOCKS5 proxies through natural language tools.
///
/// Usage:
///   PROXYBASE_API_URL=https://api.proxybase.xyz proxybase-mcp
///
/// Or for local development:
///   PROXYBASE_API_URL=http://localhost:8080 cargo run

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 Types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// MCP Tool Definitions
// ---------------------------------------------------------------------------

fn get_tools() -> Value {
    json!([
        {
            "name": "register_agent",
            "description": "Register a new AI agent with ProxyBase and receive an API key. This is the first step — you need an API key to use all other tools. The API key should be saved and reused for subsequent requests.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        },
        {
            "name": "list_packages",
            "description": "List all available proxy bandwidth packages with pricing. Each package includes a bandwidth allocation (in bytes), price (in USD), proxy type, and target country.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "api_key": {
                        "type": "string",
                        "description": "Your ProxyBase API key (starts with pk_)"
                    }
                },
                "required": ["api_key"]
            }
        },
        {
            "name": "list_currencies",
            "description": "List all available payment currencies (cryptocurrencies) that can be used for the pay_currency field when creating an order or topping up. These are the coins enabled on the payment provider's merchant account. You MUST call this before creating an order to know which pay_currency values are valid.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "api_key": {
                        "type": "string",
                        "description": "Your ProxyBase API key (starts with pk_)"
                    }
                },
                "required": ["api_key"]
            }
        },
        {
            "name": "create_order",
            "description": "Create a new proxy order. This generates a cryptocurrency payment invoice. Once payment is confirmed via the blockchain, your SOCKS5 proxy credentials will be provisioned automatically. Poll check_order_status to monitor payment and get credentials.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "api_key": {
                        "type": "string",
                        "description": "Your ProxyBase API key (starts with pk_)"
                    },
                    "package_id": {
                        "type": "string",
                        "description": "The package ID to purchase (e.g., 'us_residential_1gb')"
                    },
                    "pay_currency": {
                        "type": "string",
                        "description": "Cryptocurrency to pay with. Use list_currencies to get valid values. Defaults to 'usdttrc20'."
                    },
                    "callback_url": {
                        "type": "string",
                        "description": "Optional webhook URL to receive status notifications (payment confirmed, bandwidth 80%/95%, exhausted)"
                    }
                },
                "required": ["api_key", "package_id"]
            }
        },
        {
            "name": "check_order_status",
            "description": "Check the current status of an order. Returns payment status, bandwidth usage, and SOCKS5 proxy credentials (host:port:username:password) once the proxy is active. Statuses: payment_pending → confirming → paid → proxy_active → bandwidth_exhausted.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "api_key": {
                        "type": "string",
                        "description": "Your ProxyBase API key (starts with pk_)"
                    },
                    "order_id": {
                        "type": "string",
                        "description": "The order ID returned from create_order"
                    }
                },
                "required": ["api_key", "order_id"]
            }
        },
        {
            "name": "topup_order",
            "description": "Add more bandwidth to an existing order. Creates a new payment invoice for the additional bandwidth. The proxy credentials remain the same — only the bandwidth allowance increases. Can also reactivate an exhausted proxy.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "api_key": {
                        "type": "string",
                        "description": "Your ProxyBase API key (starts with pk_)"
                    },
                    "order_id": {
                        "type": "string",
                        "description": "The order ID to top up"
                    },
                    "package_id": {
                        "type": "string",
                        "description": "The bandwidth package to add (e.g., 'us_residential_1gb')"
                    },
                    "pay_currency": {
                        "type": "string",
                        "description": "Cryptocurrency to pay with. Use list_currencies to get valid values. Defaults to 'usdttrc20'."
                    }
                },
                "required": ["api_key", "order_id", "package_id"]
            }
        },
        {
            "name": "rotate_proxy",
            "description": "Rotate the proxy to get a fresh IP address. This calls the upstream partner's reset endpoint to invalidate the current session and assign a new IP. Only works on orders with proxy_active status. After rotation, your next SOCKS5 connection will use a new IP.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "api_key": {
                        "type": "string",
                        "description": "Your ProxyBase API key (starts with pk_)"
                    },
                    "order_id": {
                        "type": "string",
                        "description": "The order ID whose proxy should be rotated"
                    }
                },
                "required": ["api_key", "order_id"]
            }
        }
    ])
}

// ---------------------------------------------------------------------------
// ProxyBase API Client
// ---------------------------------------------------------------------------

struct ProxyBaseClient {
    http: reqwest::Client,
    base_url: String,
}

impl ProxyBaseClient {
    fn new(base_url: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    async fn register_agent(&self) -> Result<Value, String> {
        let resp = self.http
            .post(format!("{}/v1/agents", self.base_url))
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;

        let status = resp.status();
        let body: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;

        if status.is_success() {
            Ok(body)
        } else {
            Err(format!("API error ({}): {}", status, body))
        }
    }

    async fn list_packages(&self, api_key: &str) -> Result<Value, String> {
        let resp = self.http
            .get(format!("{}/v1/packages", self.base_url))
            .header("X-API-Key", api_key)
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;

        let status = resp.status();
        let body: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;

        if status.is_success() {
            Ok(body)
        } else {
            Err(format!("API error ({}): {}", status, body))
        }
    }

    async fn list_currencies(&self, api_key: &str) -> Result<Value, String> {
        let resp = self.http
            .get(format!("{}/v1/currencies", self.base_url))
            .header("X-API-Key", api_key)
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;

        let status = resp.status();
        let body: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;

        if status.is_success() {
            Ok(body)
        } else {
            Err(format!("API error ({}): {}", status, body))
        }
    }

    async fn create_order(
        &self,
        api_key: &str,
        package_id: &str,
        pay_currency: Option<&str>,
        callback_url: Option<&str>,
    ) -> Result<Value, String> {
        let mut payload = json!({ "package_id": package_id });

        if let Some(currency) = pay_currency {
            payload["pay_currency"] = json!(currency);
        }
        if let Some(url) = callback_url {
            payload["callback_url"] = json!(url);
        }

        let resp = self.http
            .post(format!("{}/v1/orders", self.base_url))
            .header("X-API-Key", api_key)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;

        let status = resp.status();
        let body: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;

        if status.is_success() {
            Ok(body)
        } else {
            Err(format!("API error ({}): {}", status, body))
        }
    }

    async fn check_order_status(&self, api_key: &str, order_id: &str) -> Result<Value, String> {
        let resp = self.http
            .get(format!("{}/v1/orders/{}/status", self.base_url, order_id))
            .header("X-API-Key", api_key)
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;

        let status = resp.status();
        let body: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;

        if status.is_success() {
            Ok(body)
        } else {
            Err(format!("API error ({}): {}", status, body))
        }
    }

    async fn topup_order(
        &self,
        api_key: &str,
        order_id: &str,
        package_id: &str,
        pay_currency: Option<&str>,
    ) -> Result<Value, String> {
        let mut payload = json!({ "package_id": package_id });

        if let Some(currency) = pay_currency {
            payload["pay_currency"] = json!(currency);
        }

        let resp = self.http
            .post(format!("{}/v1/orders/{}/topup", self.base_url, order_id))
            .header("X-API-Key", api_key)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;

        let status = resp.status();
        let body: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;

        if status.is_success() {
            Ok(body)
        } else {
            Err(format!("API error ({}): {}", status, body))
        }
    }

    async fn rotate_proxy(&self, api_key: &str, order_id: &str) -> Result<Value, String> {
        let resp = self.http
            .post(format!("{}/v1/orders/{}/rotate", self.base_url, order_id))
            .header("X-API-Key", api_key)
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;

        let status = resp.status();
        let body: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;

        if status.is_success() {
            Ok(body)
        } else {
            Err(format!("API error ({}): {}", status, body))
        }
    }
}

// ---------------------------------------------------------------------------
// MCP Request Handler
// ---------------------------------------------------------------------------

async fn handle_request(client: &ProxyBaseClient, req: &JsonRpcRequest) -> JsonRpcResponse {
    let id = req.id.clone().unwrap_or(Value::Null);

    match req.method.as_str() {
        // MCP Lifecycle
        "initialize" => JsonRpcResponse::success(id, json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "proxybase-mcp",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),

        // MCP Tool Discovery
        "tools/list" => JsonRpcResponse::success(id, json!({
            "tools": get_tools()
        })),

        // MCP Tool Execution
        "tools/call" => {
            let params = req.params.as_ref();
            let tool_name = params
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let args = params
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or(json!({}));

            let result = execute_tool(client, tool_name, &args).await;

            match result {
                Ok(content) => JsonRpcResponse::success(id, json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&content).unwrap_or_default()
                    }]
                })),
                Err(err_msg) => JsonRpcResponse::success(id, json!({
                    "content": [{
                        "type": "text",
                        "text": err_msg
                    }],
                    "isError": true
                })),
            }
        }

        // Notifications (no response needed)
        "notifications/initialized" | "notifications/cancelled" => {
            JsonRpcResponse::success(id, json!(null))
        }

        _ => JsonRpcResponse::error(id, -32601, format!("Method not found: {}", req.method)),
    }
}

async fn execute_tool(
    client: &ProxyBaseClient,
    tool_name: &str,
    args: &Value,
) -> Result<Value, String> {
    match tool_name {
        "register_agent" => client.register_agent().await,

        "list_packages" => {
            let api_key = get_str_arg(args, "api_key")?;
            client.list_packages(&api_key).await
        }

        "list_currencies" => {
            let api_key = get_str_arg(args, "api_key")?;
            client.list_currencies(&api_key).await
        }

        "create_order" => {
            let api_key = get_str_arg(args, "api_key")?;
            let package_id = get_str_arg(args, "package_id")?;
            let pay_currency = args.get("pay_currency").and_then(|v| v.as_str());

            if let Some(currency) = pay_currency {
                let currencies_val = client.list_currencies(&api_key).await?;
                if let Some(currencies_arr) = currencies_val.get("currencies").and_then(|v| v.as_array()) {
                    let valid_currencies: Vec<&str> = currencies_arr.iter().filter_map(|v| v.as_str()).collect();
                    if !valid_currencies.contains(&currency.to_lowercase().as_str()) {
                        return Err(format!("Invalid pay_currency: '{}'. Supported currencies: {}", currency, valid_currencies.join(", ")));
                    }
                }
            }

            let callback_url = args.get("callback_url").and_then(|v| v.as_str());
            client.create_order(&api_key, &package_id, pay_currency, callback_url).await
        }

        "check_order_status" => {
            let api_key = get_str_arg(args, "api_key")?;
            let order_id = get_str_arg(args, "order_id")?;
            client.check_order_status(&api_key, &order_id).await
        }

        "topup_order" => {
            let api_key = get_str_arg(args, "api_key")?;
            let order_id = get_str_arg(args, "order_id")?;
            let package_id = get_str_arg(args, "package_id")?;
            let pay_currency = args.get("pay_currency").and_then(|v| v.as_str());

            if let Some(currency) = pay_currency {
                let currencies_val = client.list_currencies(&api_key).await?;
                if let Some(currencies_arr) = currencies_val.get("currencies").and_then(|v| v.as_array()) {
                    let valid_currencies: Vec<&str> = currencies_arr.iter().filter_map(|v| v.as_str()).collect();
                    if !valid_currencies.contains(&currency.to_lowercase().as_str()) {
                        return Err(format!("Invalid pay_currency: '{}'. Supported currencies: {}", currency, valid_currencies.join(", ")));
                    }
                }
            }

            client.topup_order(&api_key, &order_id, &package_id, pay_currency).await
        }

        "rotate_proxy" => {
            let api_key = get_str_arg(args, "api_key")?;
            let order_id = get_str_arg(args, "order_id")?;
            client.rotate_proxy(&api_key, &order_id).await
        }

        _ => Err(format!("Unknown tool: {}", tool_name)),
    }
}

fn get_str_arg(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("Missing required argument: {}", key))
}

// ---------------------------------------------------------------------------
// Main: Stdio JSON-RPC Transport
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    // Logging goes to stderr so it doesn't interfere with JSON-RPC on stdout
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Stderr)
        .init();

    let base_url = std::env::var("PROXYBASE_API_URL")
        .unwrap_or_else(|_| "https://api.proxybase.xyz".to_string());

    log::info!("ProxyBase MCP Server starting (backend: {})", base_url);

    let client = ProxyBaseClient::new(&base_url);

    let stdin = io::stdin();
    let stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                log::error!("Failed to read stdin: {}", e);
                break;
            }
        };

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Parse JSON-RPC request
        let req: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                let error_resp = JsonRpcResponse::error(
                    Value::Null,
                    -32700,
                    format!("Parse error: {}", e),
                );
                let mut out = stdout.lock();
                let _ = serde_json::to_writer(&mut out, &error_resp);
                let _ = writeln!(out);
                let _ = out.flush();
                continue;
            }
        };

        // Handle request
        let response = handle_request(&client, &req).await;

        // Don't send responses for notifications (no id)
        if req.id.is_none() {
            continue;
        }

        // Write response
        let mut out = stdout.lock();
        let _ = serde_json::to_writer(&mut out, &response);
        let _ = writeln!(out);
        let _ = out.flush();
    }

    log::info!("ProxyBase MCP Server shutting down");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_tools_valid_json() {
        let tools = get_tools();
        let arr = tools.as_array().unwrap();
        assert_eq!(arr.len(), 7);

        let names: Vec<&str> = arr
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();

        assert!(names.contains(&"register_agent"));
        assert!(names.contains(&"list_packages"));
        assert!(names.contains(&"list_currencies"));
        assert!(names.contains(&"create_order"));
        assert!(names.contains(&"check_order_status"));
        assert!(names.contains(&"topup_order"));
        assert!(names.contains(&"rotate_proxy"));
    }

    #[test]
    fn test_tool_schemas_have_descriptions() {
        let tools = get_tools();
        for tool in tools.as_array().unwrap() {
            assert!(tool.get("description").is_some(), "Tool {:?} missing description", tool.get("name"));
            assert!(tool.get("inputSchema").is_some(), "Tool {:?} missing inputSchema", tool.get("name"));
        }
    }

    #[test]
    fn test_get_str_arg() {
        let args = json!({"api_key": "pk_test", "package_id": "us_1gb"});
        assert_eq!(get_str_arg(&args, "api_key").unwrap(), "pk_test");
        assert!(get_str_arg(&args, "missing").is_err());
    }

    #[test]
    fn test_jsonrpc_response_success() {
        let resp = JsonRpcResponse::success(json!(1), json!({"ok": true}));
        let serialized = serde_json::to_value(&resp).unwrap();
        assert_eq!(serialized["jsonrpc"], "2.0");
        assert_eq!(serialized["id"], 1);
        assert!(serialized.get("error").is_none());
    }

    #[test]
    fn test_jsonrpc_response_error() {
        let resp = JsonRpcResponse::error(json!(1), -32600, "Invalid request");
        let serialized = serde_json::to_value(&resp).unwrap();
        assert_eq!(serialized["error"]["code"], -32600);
        assert_eq!(serialized["error"]["message"], "Invalid request");
        assert!(serialized.get("result").is_none());
    }

    #[tokio::test]
    async fn test_handle_initialize() {
        let client = ProxyBaseClient::new("http://localhost:9999");
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "initialize".to_string(),
            params: None,
        };

        let resp = handle_request(&client, &req).await;
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert!(result["capabilities"]["tools"].is_object());
        assert_eq!(result["serverInfo"]["name"], "proxybase-mcp");
    }

    #[tokio::test]
    async fn test_handle_tools_list() {
        let client = ProxyBaseClient::new("http://localhost:9999");
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(2)),
            method: "tools/list".to_string(),
            params: None,
        };

        let resp = handle_request(&client, &req).await;
        let result = resp.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 7);
    }

    #[tokio::test]
    async fn test_handle_unknown_method() {
        let client = ProxyBaseClient::new("http://localhost:9999");
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(3)),
            method: "unknown/method".to_string(),
            params: None,
        };

        let resp = handle_request(&client, &req).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }

    #[tokio::test]
    async fn test_handle_tools_call_missing_arg() {
        let client = ProxyBaseClient::new("http://localhost:9999");
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(4)),
            method: "tools/call".to_string(),
            params: Some(json!({
                "name": "list_packages",
                "arguments": {}
            })),
        };

        let resp = handle_request(&client, &req).await;
        let result = resp.result.unwrap();
        assert_eq!(result["isError"], true);
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Missing required argument: api_key"));
    }
}
