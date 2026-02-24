# ProxyBase MCP Server

A [Model Context Protocol](https://modelcontextprotocol.io) (MCP) server that lets AI agents purchase and manage SOCKS5 proxies programmatically through [ProxyBase](https://proxybase.xyz).

## Installation

### Download Pre-built Binary

Grab the latest binary for your platform from [Releases](https://github.com/proxybasehq/proxybase-mcp/releases):

| Platform | Binary |
|---|---|
| Linux x86_64 | `proxybase-mcp-linux-x86_64` |
| Linux aarch64 | `proxybase-mcp-linux-aarch64` |
| macOS x86_64 | `proxybase-mcp-macos-x86_64` |
| macOS Apple Silicon | `proxybase-mcp-macos-aarch64` |
| Windows x86_64 | `proxybase-mcp-windows-x86_64.exe` |

### Build from Source

```bash
git clone https://github.com/proxybasehq/proxybase-mcp.git
cd proxybase-mcp
cargo build --release
# Binary at: target/release/proxybase-mcp
```

## Configuration

| Environment Variable | Default | Description |
|---|---|---|
| `PROXYBASE_API_URL` | `https://api.proxybase.xyz` | ProxyBase backend URL |
| `RUST_LOG` | `info` | Log level (logs go to stderr) |

## MCP Client Setup

Add to your MCP client config:

### Claude Desktop / Cursor

```json
{
  "mcpServers": {
    "proxybase": {
      "command": "/path/to/proxybase-mcp"
    }
  }
}
```

### With Custom Backend URL

```json
{
  "mcpServers": {
    "proxybase": {
      "command": "/path/to/proxybase-mcp",
      "env": {
        "PROXYBASE_API_URL": "https://api.proxybase.xyz"
      }
    }
  }
}
```

## Available Tools

### `register_agent`
Register a new AI agent and receive an API key. **Always the first step.**

**Parameters:** None

**Returns:**
```json
{
  "agent_id": "6xAMqAGN",
  "api_key": "pk_c8c91c8a0e5b3e2c..."
}
```

---

### `list_packages`
List available proxy bandwidth packages with pricing.

| Param | Required | Description |
|---|---|---|
| `api_key` | ✅ | Your API key (starts with `pk_`) |

---

### `list_currencies`
List available payment currencies (cryptocurrencies) for the `pay_currency` field.

| Param | Required | Description |
|---|---|---|
| `api_key` | ✅ | Your API key (starts with `pk_`) |

**Returns:**
```json
{
  "currencies": ["btc", "eth", "sol", "usdttrc20", "ltc", ...]
}
```

---

### `create_order`
Purchase a proxy package. Generates a cryptocurrency payment invoice.

| Param | Required | Description |
|---|---|---|
| `api_key` | ✅ | Your API key |
| `package_id` | ✅ | Package to purchase (e.g., `us_residential_1gb`) |
| `pay_currency` | | Crypto to pay with (default: `usdttrc20`). Use `list_currencies` for valid values |
| `callback_url` | | Webhook URL for status notifications |

**Returns:**
```json
{
  "order_id": "kQx7p3Wn",
  "payment_id": "5832461907",
  "pay_address": "TXyz...",
  "pay_currency": "usdttrc20",
  "pay_amount": 10.15,
  "price_usd": 10.00,
  "status": "payment_pending"
}
```

---

### `check_order_status`
Poll order status. Returns proxy credentials once active.

| Param | Required | Description |
|---|---|---|
| `api_key` | ✅ | Your API key |
| `order_id` | ✅ | Order ID from `create_order` |

**Returns** (when proxy is active):
```json
{
  "order_id": "kQx7p3Wn",
  "status": "proxy_active",
  "bandwidth_bytes": 1073741824,
  "used_bytes": 52428800,
  "remaining_bytes": 1021313024,
  "usage_percentage": 4.88,
  "proxy": {
    "host": "api.proxybase.xyz",
    "port": 1080,
    "username": "pb_a1b2c3d4e5f6g7h8",
    "password": "9f8e7d6c5b4a3210"
  }
}
```

**Status Flow:** `payment_pending` → `confirming` → `paid` → `proxy_active` → `bandwidth_exhausted`

---

### `topup_order`
Add bandwidth to an existing proxy. Same credentials, more bandwidth.

| Param | Required | Description |
|---|---|---|
| `api_key` | ✅ | Your API key |
| `order_id` | ✅ | Order to top up |
| `package_id` | ✅ | Bandwidth package to add |
| `pay_currency` | | Crypto to pay with. Use `list_currencies` for valid values |

---

## Typical Agent Workflow

```
1. register_agent      → Save your api_key
2. list_packages       → Choose a package
3. list_currencies     → See valid pay_currency values
4. create_order        → Get payment address
5. [Pay via blockchain]
6. check_order_status  → Poll until status = "proxy_active"
7. Use proxy: socks5://username:password@api.proxybase.xyz:1080
8. check_order_status  → Monitor bandwidth usage
9. topup_order         → When bandwidth runs low
```

## Protocol Details

- **Transport:** stdio (JSON-RPC 2.0, one message per line)
- **MCP Version:** `2024-11-05`
- **Capabilities:** `tools` only

## Testing

```bash
cargo test

# Manual: send JSON-RPC over stdio
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | cargo run 2>/dev/null
```

## License

MIT
