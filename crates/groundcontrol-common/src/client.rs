//! Client tracking, authentication, and visual identity configuration.
//!
//! Provides multi-agent tracking and color association for MCP sessions and
//! the 3D GraphView visualizer. Supports optional local network client-key auth.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Visual and authentication profile for a connected AI client / agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientEntry {
    /// Unique identifier for the client (e.g. "antigravity", "claude", "gemini").
    pub id: String,
    /// Human-friendly display name (e.g. "Antigravity Agent", "Claude Desktop").
    pub name: String,
    /// Optional authentication secret key for local network verification.
    #[serde(default)]
    pub key: Option<String>,
    /// Hex color code associated with this agent (e.g. "#38bdf8").
    #[serde(default = "default_client_color")]
    pub color: String,
}

fn default_client_color() -> String {
    "#38bdf8".to_string()
}

/// Registry of known clients and authentication rules.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientsRegistry {
    /// List of registered client profiles.
    #[serde(default)]
    pub clients: Vec<ClientEntry>,
    /// Whether incoming MCP requests must provide a valid matching key.
    #[serde(default)]
    pub require_auth: bool,
    /// Optional dedicated authentication secret key for daemon-to-graphview relay.
    #[serde(default)]
    pub daemon_key: Option<String>,
}

impl Default for ClientsRegistry {
    fn default() -> Self {
        Self {
            clients: vec![
                ClientEntry {
                    id: "antigravity".to_string(),
                    name: "Antigravity Agent".to_string(),
                    key: None,
                    color: "#38bdf8".to_string(), // Cyan
                },
                ClientEntry {
                    id: "claude".to_string(),
                    name: "Claude Desktop".to_string(),
                    key: None,
                    color: "#f97316".to_string(), // Orange
                },
                ClientEntry {
                    id: "gemini".to_string(),
                    name: "Gemini CLI".to_string(),
                    key: None,
                    color: "#ec4899".to_string(), // Magenta
                },
                ClientEntry {
                    id: "roo".to_string(),
                    name: "Roo Code".to_string(),
                    key: None,
                    color: "#10b981".to_string(), // Emerald
                },
                ClientEntry {
                    id: "default".to_string(),
                    name: "Anonymous Agent".to_string(),
                    key: None,
                    color: "#a855f7".to_string(), // Purple
                },
            ],
            require_auth: false,
            daemon_key: None,
        }
    }
}

impl ClientsRegistry {
    /// Check whether a secret key matches any configured client or the daemon relay key.
    pub fn is_valid_key(&self, key: &str) -> bool {
        if key.is_empty() {
            return false;
        }
        self.find_by_key(key).is_some() || self.daemon_key.as_deref() == Some(key)
    }

    /// Find a client entry by its secret key.
    pub fn find_by_key(&self, key: &str) -> Option<&ClientEntry> {
        if key.is_empty() {
            return None;
        }
        self.clients.iter().find(|c| c.key.as_deref() == Some(key))
    }

    /// Find a client entry by its unique identifier (case-insensitive).
    pub fn find_by_id(&self, id: &str) -> Option<&ClientEntry> {
        let lower = id.to_lowercase();
        self.clients.iter().find(|c| c.id.to_lowercase() == lower)
    }

    /// Resolve a client from an optional key or ID, falling back to default or anonymous.
    pub fn resolve(&self, key: Option<&str>, id: Option<&str>) -> Option<&ClientEntry> {
        if let Some(k) = key {
            if let Some(entry) = self.find_by_key(k) {
                return Some(entry);
            }
        }
        if let Some(i) = id {
            if let Some(entry) = self.find_by_id(i) {
                return Some(entry);
            }
        }
        self.find_by_id("default").or_else(|| self.clients.first())
    }
}

/// Generate a cryptographically hashed token with a given prefix.
pub fn generate_token(prefix: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let mut hasher = blake3::Hasher::new();
    hasher.update(&now.to_le_bytes());
    hasher.update(&pid.to_le_bytes());
    hasher.update(prefix.as_bytes());
    let hex = hasher.finalize().to_hex();
    format!("{prefix}_{}", &hex[..24])
}

/// Generate a fresh `ClientsRegistry` populated with secure random API keys and `require_auth: false`.
pub fn generate_default_config() -> ClientsRegistry {
    ClientsRegistry {
        clients: vec![
            ClientEntry {
                id: "antigravity".to_string(),
                name: "Antigravity Agent".to_string(),
                key: Some(generate_token("ag")),
                color: "#38bdf8".to_string(), // Cyan
            },
            ClientEntry {
                id: "claude".to_string(),
                name: "Claude Desktop".to_string(),
                key: Some(generate_token("claude")),
                color: "#f97316".to_string(), // Orange
            },
            ClientEntry {
                id: "gemini".to_string(),
                name: "Gemini CLI".to_string(),
                key: Some(generate_token("gemini")),
                color: "#ec4899".to_string(), // Magenta
            },
            ClientEntry {
                id: "cursor".to_string(),
                name: "Cursor".to_string(),
                key: Some(generate_token("cursor")),
                color: "#10b981".to_string(), // Emerald
            },
            ClientEntry {
                id: "windsurf".to_string(),
                name: "Windsurf".to_string(),
                key: Some(generate_token("windsurf")),
                color: "#06b6d4".to_string(), // Teal
            },
            ClientEntry {
                id: "vscode".to_string(),
                name: "VS Code Copilot".to_string(),
                key: Some(generate_token("vscode")),
                color: "#3b82f6".to_string(), // Blue
            },
            ClientEntry {
                id: "zed".to_string(),
                name: "Zed".to_string(),
                key: Some(generate_token("zed")),
                color: "#eab308".to_string(), // Yellow
            },
            ClientEntry {
                id: "roo".to_string(),
                name: "Roo Code".to_string(),
                key: Some(generate_token("roo")),
                color: "#14b8a6".to_string(), // Mint
            },
            ClientEntry {
                id: "kiro".to_string(),
                name: "Kiro CLI".to_string(),
                key: Some(generate_token("kiro")),
                color: "#8b5cf6".to_string(), // Violet
            },
            ClientEntry {
                id: "default".to_string(),
                name: "Anonymous Agent".to_string(),
                key: None,
                color: "#a855f7".to_string(), // Purple
            },
        ],
        require_auth: false,
        daemon_key: Some(generate_token("daemon")),
    }
}

impl ClientsRegistry {
    /// Retrieve the key for a given client ID, generating and persisting a new one if missing.
    pub fn get_or_create_client_key(&mut self, client_id: &str) -> String {
        let lower = client_id.to_lowercase();
        if let Some(entry) = self.clients.iter_mut().find(|c| c.id.to_lowercase() == lower) {
            if let Some(ref k) = entry.key {
                return k.clone();
            }
            let new_key = generate_token(&lower);
            entry.key = Some(new_key.clone());
            return new_key;
        }

        let new_key = generate_token(&lower);
        self.clients.push(ClientEntry {
            id: client_id.to_string(),
            name: format!("{client_id} Client"),
            key: Some(new_key.clone()),
            color: default_client_color(),
        });
        new_key
    }
}

/// Get the canonical path for central `clients.json`.
pub fn get_central_clients_path() -> PathBuf {
    crate::config::get_cache_dir().join("clients.json")
}

/// Discover candidate paths for `clients.json`.
pub fn get_client_config_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(path_str) = std::env::var("GROUNDCONTROL_CLIENTS_CONFIG")
        .or_else(|_| std::env::var("GC_CLIENTS_CONFIG"))
        .or_else(|_| std::env::var("CTXV_CLIENTS_CONFIG"))
    {
        if !path_str.is_empty() {
            candidates.push(PathBuf::from(path_str));
        }
    }

    candidates.push(PathBuf::from("clients.json"));
    candidates.push(PathBuf::from("gc-clients.json"));
    candidates.push(PathBuf::from("ctxv-clients.json"));
    candidates.push(get_central_clients_path());

    candidates
}

/// Save a `ClientsRegistry` configuration to a target path on disk.
pub fn save_clients_config(registry: &ClientsRegistry, path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json_str = serde_json::to_string_pretty(registry)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, json_str)?;
    Ok(())
}

/// Ensure central authentication and clients configuration exists in central config.toml.
///
/// If `generate_if_missing` is true, generates a fresh `ClientsRegistry` populated with
/// secure API keys for all supported agents and writes it to central configuration.
pub fn ensure_central_clients_config(
    generate_if_missing: bool,
    require_auth: bool,
) -> std::io::Result<(ClientsRegistry, PathBuf)> {
    let config_path = crate::config::get_config_path();
    let mut global_cfg = crate::config::load_global_config();

    let mut changed = false;
    if global_cfg.auth.clients.is_empty() && generate_if_missing {
        global_cfg.auth = generate_default_config();
        changed = true;
    }
    if require_auth && !global_cfg.auth.require_auth {
        global_cfg.auth.require_auth = true;
        changed = true;
    }
    if global_cfg.auth.daemon_key.is_none() {
        global_cfg.auth.daemon_key = Some(generate_token("daemon"));
        changed = true;
    }
    if global_cfg.graphview.daemon_key != global_cfg.auth.daemon_key {
        global_cfg.graphview.daemon_key = global_cfg.auth.daemon_key.clone();
        changed = true;
    }
    if changed {
        let _ = crate::config::save_global_config(&global_cfg);
    }
    Ok((global_cfg.auth, config_path))
}

/// Load the clients registry from disk, falling back to built-in defaults.
pub fn load_clients_config(explicit_path: Option<&Path>) -> ClientsRegistry {
    let mut registry = if let Some(path) = explicit_path {
        if path.exists() {
            std::fs::read_to_string(path)
                .ok()
                .and_then(|c| serde_json::from_str::<ClientsRegistry>(&c).ok())
        } else {
            None
        }
    } else {
        None
    };

    if registry.is_none() {
        let global_cfg = crate::config::load_global_config();
        if !global_cfg.auth.clients.is_empty()
            || global_cfg.auth.require_auth
            || global_cfg.auth.daemon_key.is_some()
        {
            registry = Some(global_cfg.auth);
        }
    }

    if registry.is_none() {
        for candidate in get_client_config_candidates() {
            if candidate.exists() {
                if let Ok(content) = std::fs::read_to_string(&candidate) {
                    if let Ok(reg) = serde_json::from_str::<ClientsRegistry>(&content) {
                        registry = Some(reg);
                        break;
                    }
                }
            }
        }
    }

    let mut reg = registry.unwrap_or_default();

    // Check environment variable overrides
    if let Ok(val) = std::env::var("GROUNDCONTROL_REQUIRE_AUTH")
        .or_else(|_| std::env::var("GC_REQUIRE_AUTH"))
        .or_else(|_| std::env::var("CTXV_REQUIRE_AUTH"))
    {
        if val == "1" || val.eq_ignore_ascii_case("true") {
            reg.require_auth = true;
        } else if val == "0" || val.eq_ignore_ascii_case("false") {
            reg.require_auth = false;
        }
    }

    if let Ok(key) = std::env::var("GROUNDCONTROL_INTERNAL_API_KEY")
        .or_else(|_| std::env::var("GC_INTERNAL_API_KEY"))
        .or_else(|_| std::env::var("GROUNDCONTROL_DAEMON_KEY"))
        .or_else(|_| std::env::var("GC_DAEMON_KEY"))
        .or_else(|_| std::env::var("CTXV_INTERNAL_API_KEY"))
        .or_else(|_| std::env::var("CTXV_DAEMON_KEY"))
    {
        if !key.trim().is_empty() {
            reg.daemon_key = Some(key.trim().to_string());
        }
    }

    reg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_registry_no_auth() {
        let registry = ClientsRegistry::default();
        assert!(!registry.require_auth);
        assert_eq!(registry.daemon_key, None);
        assert!(registry.clients.len() >= 4);

        let ag = registry.find_by_id("antigravity").unwrap();
        assert_eq!(ag.name, "Antigravity Agent");
        assert_eq!(ag.color, "#38bdf8");
    }

    #[test]
    fn test_generate_default_config_has_keys() {
        let generated = generate_default_config();
        assert!(!generated.require_auth);
        assert!(generated.daemon_key.is_some());
        let daemon_k = generated.daemon_key.as_ref().unwrap();
        assert!(daemon_k.starts_with("daemon_"));
        assert!(generated.is_valid_key(daemon_k));

        let ag = generated.find_by_id("antigravity").unwrap();
        let ag_key = ag.key.as_ref().unwrap();
        assert!(ag_key.starts_with("ag_"));
        assert!(generated.is_valid_key(ag_key));
    }

    #[test]
    fn test_resolve_key_and_id() {
        let mut reg = ClientsRegistry::default();
        reg.clients[0].key = Some("test-secret-123".to_string());

        let resolved = reg.resolve(Some("test-secret-123"), None).unwrap();
        assert_eq!(resolved.id, "antigravity");

        let resolved_id = reg.resolve(None, Some("claude")).unwrap();
        assert_eq!(resolved_id.id, "claude");

        let resolved_default = reg.resolve(None, None).unwrap();
        assert_eq!(resolved_default.id, "default");
    }

    #[test]
    fn test_expanded_clients_in_default_config() {
        let generated = generate_default_config();
        for id in &[
            "antigravity",
            "claude",
            "gemini",
            "cursor",
            "windsurf",
            "vscode",
            "zed",
            "roo",
            "kiro",
        ] {
            let client = generated.find_by_id(id);
            assert!(client.is_some(), "Client {id} should exist in default config");
            let k = client.unwrap().key.as_ref();
            assert!(k.is_some(), "Client {id} should have an auto-generated key");
            assert!(generated.is_valid_key(k.unwrap()));
        }
    }

    #[test]
    fn test_get_or_create_client_key() {
        let mut reg = ClientsRegistry::default();
        let key1 = reg.get_or_create_client_key("cursor");
        assert!(key1.starts_with("cursor_"));
        assert!(reg.is_valid_key(&key1));

        // Calling again returns the identical existing key
        let key2 = reg.get_or_create_client_key("cursor");
        assert_eq!(key1, key2);

        // A new unrecognized agent gets added
        let custom_key = reg.get_or_create_client_key("my-custom-agent");
        assert!(custom_key.starts_with("my-custom-agent_"));
        assert!(reg.find_by_id("my-custom-agent").is_some());
    }
}
