//! Auto-configuration and installation of ctxvault for coding agents.
//!
//! Detects installed coding agents across platform standard paths, configures
//! `mcpServers` with a zero-arg `ctxvault` launcher entry, and optionally
//! auto-populates up-to-date agent steering rules.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Default authoritative steering rule content for AI agents interacting with ctxvault.
pub const CTXVAULT_STEERING_RULE: &str = r#"# ctxvault MCP Steering Protocol

You have access to the `ctxvault` Model Context Protocol (MCP) server (17 authoritative tools). Follow these rules when querying or modifying knowledge and source code:

1. Retrieval Strategy & Turn 1 Snippets:
   - Use `search` with `mode="hybrid"` as your default exploratory discovery tool (3-way RRF across BM25 lexical, ONNX dense vectors, and graph).
   - `search` automatically returns Turn 1 inline text and code snippets (`snippets: 3` by default) along with graph affordances (`calls_in`, `calls_out`, `implements`, `imports`, `wikilinks_in`). Work directly from these snippets whenever possible to avoid unnecessary round-trips.
   - For fast bare-identifier or file handle sweeps, pass `detail="ids"` to strip snippets, affordances, and score breakdowns for minimum token footprint (<250 tokens).
   - For quick structural repository census or architecture overviews, call `status(scope="census")` or `status(scope="architecture")` for instant counts of symbols, edges, languages, and files (<2ms).
   - Use `search` with `mode="bm25"` when searching for exact identifier names, error strings, struct symbols, or CLI flags.
   - Use `search` with `mode="semantic"` for abstract natural-language concepts.
   - Use `search_related` for Personalized PageRank expansion around known seed notes or symbols.

2. Bounded Code & Document Inspection (Progressive Disclosure):
   - Tier 1: Survey results via `search` (inspect handles, Turn 1 snippets, and graph affordances).
   - Tier 2: Fetch exact symbol definitions or bounded doc chunks with `get_snippet(symbol="...")` or `get_snippet(path="...", chunk_index=0)`.
   - Tier 3: Call `read_file` with explicit line bounds (`start_line`, `end_line`) only when exhaustive context is required. Do NOT dump entire large files into context.

3. Graph Navigation with Cypher-Lite (`graph_match`):
   - Call `graph_match` using linear Cypher-Lite ASCII patterns with cycle guards:
     - Trace call graphs: `(:CodeSymbol {name: "MyFunction"})-[:calls*1..2]->(target)`
     - Trace implementations: `(source)-[:implements]->(target)`
     - Trace doc ancestry: `(:DocNode {path: "adrs/001.md"})-[:derived_from*1..3]->(target)`
   - Filter by `edge_class`: `"code"`, `"structural"`, `"semantic"`, `"crossmodal"`, or `"hybrid"`.
   - Use `graph_communities(view="architecture")` for high-level architectural component maps.

4. Note Creation & Schema Discipline:
   - Before authoring a new document or ADR, call `list_templates` to discover available schemas.
   - Author or update notes via `write_note(path="...", mode="create"|"overwrite"|"append")`.
   - Always run `validate(path="...")` immediately after creating or modifying a note to ensure zero schema errors.

5. Principle 3 Knowledge Crystallization:
   - When resolving complex architectural questions, subtle bugs, or incident resolutions, crystallize findings into permanent notes using `write_note` (with `derived_from` frontmatter) and verify lineage via `graph_match`.
"#;

/// Configuration target for a coding agent.
#[derive(Debug, Clone)]
pub struct AgentTarget {
    /// Human-readable agent name.
    pub name: &'static str,
    /// Registered client identity (e.g. "antigravity", "claude", "cursor").
    pub client_id: &'static str,
    /// Absolute path to configuration file.
    pub path: PathBuf,
}

/// Steering rule target for a coding agent.
#[derive(Debug, Clone)]
pub struct RuleTarget {
    /// Human-readable target name.
    pub name: &'static str,
    /// Absolute path to the rule file.
    pub path: PathBuf,
}

/// Discovered agent installation and configuration status.
#[derive(Debug, Default)]
pub struct InstallSummary {
    /// Agents successfully configured with MCP servers.
    pub configured: Vec<String>,
    /// Agents detected during dry-run.
    pub dry_run_detected: Vec<String>,
    /// Agents skipped because app is not installed.
    pub skipped: Vec<String>,
    /// Steering rule files successfully created or updated.
    pub rules_configured: Vec<String>,
}

/// Detect configuration locations for all supported coding agents on the current OS.
pub fn detect_agents() -> Vec<AgentTarget> {
    let mut targets = Vec::new();

    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    let home_path = PathBuf::from(&home);

    #[cfg(windows)]
    let app_data = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_path.join("AppData").join("Roaming"));

    #[cfg(not(windows))]
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_path.join(".config"));

    // 1. Antigravity / Gemini
    targets.push(AgentTarget {
        name: "Antigravity IDE",
        client_id: "antigravity",
        path: home_path.join(".gemini").join("antigravity-ide").join("mcp_config.json"),
    });
    targets.push(AgentTarget {
        name: "Gemini CLI / Extension",
        client_id: "gemini",
        path: home_path.join(".gemini").join("config").join("mcp_config.json"),
    });

    // 2. Cursor
    #[cfg(windows)]
    {
        targets.push(AgentTarget {
            name: "Cursor (Global Settings)",
            client_id: "cursor",
            path: app_data.join("Cursor").join("User").join("mcp.json"),
        });
        targets.push(AgentTarget {
            name: "Cursor (Roo/Cline MCP)",
            client_id: "roo",
            path: app_data
                .join("Cursor")
                .join("User")
                .join("globalStorage")
                .join("rooveterinaryinc.roo-cline")
                .join("settings")
                .join("cline_mcp_settings.json"),
        });
    }
    #[cfg(not(windows))]
    {
        targets.push(AgentTarget {
            name: "Cursor",
            client_id: "cursor",
            path: config_dir.join("Cursor").join("User").join("mcp.json"),
        });
    }
    targets.push(AgentTarget {
        name: "Cursor (User Profile)",
        client_id: "cursor",
        path: home_path.join(".cursor").join("mcp.json"),
    });

    // 3. Claude Desktop & Claude Code
    #[cfg(windows)]
    {
        targets.push(AgentTarget {
            name: "Claude Desktop",
            client_id: "claude",
            path: app_data.join("Claude").join("claude_desktop_config.json"),
        });
    }
    #[cfg(not(windows))]
    {
        targets.push(AgentTarget {
            name: "Claude Desktop",
            client_id: "claude",
            path: home_path.join(".claude").join("claude_desktop_config.json"),
        });
    }
    targets.push(AgentTarget {
        name: "Claude Code CLI",
        client_id: "claude",
        path: home_path.join(".claude.json"),
    });

    // 4. Windsurf
    targets.push(AgentTarget {
        name: "Windsurf",
        client_id: "windsurf",
        path: home_path.join(".codeium").join("windsurf").join("mcp_config.json"),
    });

    // 5. VS Code / Copilot
    #[cfg(windows)]
    {
        targets.push(AgentTarget {
            name: "VS Code User MCP",
            client_id: "vscode",
            path: app_data.join("Code").join("User").join("mcp.json"),
        });
        targets.push(AgentTarget {
            name: "GitHub Copilot Chat MCP",
            client_id: "vscode",
            path: app_data
                .join("Code")
                .join("User")
                .join("globalStorage")
                .join("github.copilot-chat")
                .join("mcp.json"),
        });
    }
    #[cfg(not(windows))]
    {
        targets.push(AgentTarget {
            name: "VS Code User MCP",
            client_id: "vscode",
            path: config_dir.join("Code").join("User").join("mcp.json"),
        });
    }

    // 6. Zed
    #[cfg(windows)]
    {
        targets.push(AgentTarget {
            name: "Zed",
            client_id: "zed",
            path: app_data.join("Zed").join("settings.json"),
        });
    }
    #[cfg(not(windows))]
    {
        targets.push(AgentTarget {
            name: "Zed",
            client_id: "zed",
            path: config_dir.join("zed").join("settings.json"),
        });
    }

    // 7. Kiro CLI
    let kiro_home =
        std::env::var("KIRO_HOME").map(PathBuf::from).unwrap_or_else(|_| home_path.join(".kiro"));
    targets.push(AgentTarget {
        name: "Kiro CLI (Global Settings)",
        client_id: "kiro",
        path: kiro_home.join("settings").join("mcp.json"),
    });

    targets
}

/// Detect steering rule targets for coding agents and workspace.
pub fn detect_rule_targets(workspace_dir: Option<&Path>) -> Vec<RuleTarget> {
    let mut targets = Vec::new();

    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    let home_path = PathBuf::from(&home);

    // 1. Antigravity IDE global rules
    let antigravity_rules =
        home_path.join(".gemini").join("antigravity-ide").join("rules").join("ctxvault.md");
    targets.push(RuleTarget { name: "Antigravity Global Rules", path: antigravity_rules });

    // 2. Gemini CLI global rules
    let gemini_rules = home_path.join(".gemini").join("config").join("rules").join("ctxvault.md");
    targets.push(RuleTarget { name: "Gemini CLI Global Rules", path: gemini_rules });

    // 3. Cursor global rules
    let cursor_rules = home_path.join(".cursor").join("rules").join("ctxvault.mdc");
    targets.push(RuleTarget { name: "Cursor Global Rules", path: cursor_rules });

    // 4. Claude Code home instructions
    let claude_rules = home_path.join(".claude").join("CLAUDE.md");
    targets.push(RuleTarget { name: "Claude Code Home Rules", path: claude_rules });

    // 5. Kiro CLI global rules & steering
    let kiro_home =
        std::env::var("KIRO_HOME").map(PathBuf::from).unwrap_or_else(|_| home_path.join(".kiro"));
    targets.push(RuleTarget {
        name: "Kiro CLI Global Rules",
        path: kiro_home.join("rules").join("ctxvault.md"),
    });
    targets.push(RuleTarget {
        name: "Kiro CLI Global Steering",
        path: kiro_home.join("steering").join("ctxvault.md"),
    });

    // 6. Workspace-specific rules if workspace_dir provided
    if let Some(ws) = workspace_dir {
        targets.push(RuleTarget { name: "Workspace GEMINI.md", path: ws.join("GEMINI.md") });
        targets.push(RuleTarget { name: "Workspace .cursorrules", path: ws.join(".cursorrules") });
        targets
            .push(RuleTarget { name: "Workspace .windsurfrules", path: ws.join(".windsurfrules") });
        targets.push(RuleTarget {
            name: "Workspace Kiro Rules",
            path: ws.join(".kiro").join("rules").join("ctxvault.md"),
        });
        targets.push(RuleTarget {
            name: "Workspace Kiro Steering",
            path: ws.join(".kiro").join("steering").join("ctxvault.md"),
        });
    }

    targets
}

/// Run auto-configuration across detected agents and optionally populate steering rules.
pub fn run_install(
    install_dir: Option<&Path>,
    dry_run: bool,
    _auto_confirm: bool,
    install_rules: bool,
    workspace_dir: Option<&Path>,
    filter_agents: Option<&[String]>,
    with_auth: bool,
) -> anyhow::Result<InstallSummary> {
    let binary_command = if let Some(dir) = install_dir {
        let exe = if cfg!(windows) { "ctxvault.exe" } else { "ctxvault" };
        dir.join(exe).to_string_lossy().to_string()
    } else {
        "ctxvault".to_string()
    };

    let mut clients_registry = if with_auth {
        let (reg, _) = ctxvault_common::client::ensure_central_clients_config(true, false)?;
        Some(reg)
    } else {
        None
    };

    let mut targets = detect_agents();
    if let Some(ws) = workspace_dir {
        targets.push(AgentTarget {
            name: "Kiro CLI (Workspace Settings)",
            client_id: "kiro",
            path: ws.join(".kiro").join("settings").join("mcp.json"),
        });
    }

    if let Some(filters) = filter_agents {
        targets.retain(|t| {
            let lower = t.name.to_lowercase();
            filters.iter().any(|f| lower.contains(&f.to_lowercase()))
        });
    }

    let mut summary = InstallSummary::default();

    for target in targets {
        let file_exists = target.path.exists();
        let parent_exists = target.path.parent().map(|p| p.exists()).unwrap_or(false);

        // Only configure if either the config file already exists or its parent app directory exists.
        if !file_exists && !parent_exists {
            summary.skipped.push(format!("{} (app not detected)", target.name));
            continue;
        }

        let mut root_val: Value = if file_exists {
            match fs::read_to_string(&target.path) {
                Ok(content) => serde_json::from_str(&content).unwrap_or(json!({})),
                Err(_) => json!({}),
            }
        } else {
            json!({})
        };

        if !root_val.is_object() {
            root_val = json!({});
        }

        let root_map = root_val.as_object_mut().unwrap();

        // Ensure mcpServers object exists
        let servers = root_map.entry("mcpServers".to_string()).or_insert_with(|| json!({}));

        let mut server_entry = json!({
            "command": binary_command,
            "args": []
        });

        if let Some(ref mut reg) = clients_registry {
            let key = reg.get_or_create_client_key(target.client_id);
            server_entry.as_object_mut().unwrap().insert(
                "env".to_string(),
                json!({
                    "CTXV_API_KEY": key
                }),
            );
        }

        if let Some(servers_map) = servers.as_object_mut() {
            servers_map.insert("ctxvault".to_string(), server_entry);
        }

        let target_display = target.path.display().to_string();

        if dry_run {
            let auth_note = if with_auth { " (with CTXV_API_KEY credentials)" } else { "" };
            summary.dry_run_detected.push(format!(
                "{} -> Would configure ctxvault{} at: {}",
                target.name, auth_note, target_display
            ));
        } else {
            if let Some(parent) = target.path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let pretty_json = serde_json::to_string_pretty(&root_val)?;
            fs::write(&target.path, pretty_json)?;
            summary.configured.push(format!("{} [{}]", target.name, target_display));
        }
    }

    // Auto-populate steering rules if requested
    if install_rules {
        let mut rule_targets = detect_rule_targets(workspace_dir);
        if let Some(filters) = filter_agents {
            rule_targets.retain(|rt| {
                let lower = rt.name.to_lowercase();
                filters.iter().any(|f| lower.contains(&f.to_lowercase()))
            });
        }
        for rt in rule_targets {
            let parent_exists = rt.path.parent().map(|p| p.exists()).unwrap_or(false);
            if !parent_exists && workspace_dir.is_none() {
                // If it's a global agent path and the parent agent directory does not exist, skip
                continue;
            }

            let path_display = rt.path.display().to_string();
            if dry_run {
                summary
                    .dry_run_detected
                    .push(format!("{} -> Would write steering rule to: {}", rt.name, path_display));
            } else {
                if let Some(parent) = rt.path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::write(&rt.path, CTXVAULT_STEERING_RULE)?;
                summary.rules_configured.push(format!("{} [{}]", rt.name, path_display));
            }
        }
    }

    // Configure Kiro subagent profiles (scout & analysis) if Kiro is detected
    let kiro_dirs = {
        let mut dirs = Vec::new();
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".to_string());
        let kiro_home = std::env::var("KIRO_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(&home).join(".kiro"));
        if kiro_home.exists() {
            dirs.push(kiro_home.join("agents"));
        }
        if let Some(ws) = workspace_dir {
            let ws_kiro = ws.join(".kiro");
            if ws_kiro.exists() {
                dirs.push(ws_kiro.join("agents"));
            }
        }
        dirs
    };

    for agents_dir in kiro_dirs {
        let mut scout_server = json!({
            "command": binary_command,
            "args": ["--profile", "scout"]
        });
        let mut analysis_server = json!({
            "command": binary_command,
            "args": ["--profile", "analysis"]
        });

        if let Some(ref mut reg) = clients_registry {
            let key = reg.get_or_create_client_key("kiro");
            let env_map = json!({ "CTXV_API_KEY": key });
            scout_server.as_object_mut().unwrap().insert("env".to_string(), env_map.clone());
            analysis_server.as_object_mut().unwrap().insert("env".to_string(), env_map);
        }

        let scout_profile = json!({
            "name": "ctxvault-scout",
            "description": "Fast exploratory code & doc scout agent using ctxvault scout profile",
            "prompt": "You are a lightweight code scout. Use ctxvault tools (search, get_snippet, status) for high-signal retrieval without full file dumps.",
            "tools": ["read", "grep", "glob"],
            "includeMcpJson": false,
            "mcpServers": {
                "ctxvault": scout_server
            }
        });
        let analysis_profile = json!({
            "name": "ctxvault-analysis",
            "description": "Deep architectural & graph analysis agent using ctxvault analysis profile",
            "prompt": "You are an architectural analyst. Use ctxvault graph_match, graph_communities, search, and validate for system mapping and dependency tracing.",
            "tools": ["read", "grep", "glob"],
            "includeMcpJson": false,
            "mcpServers": {
                "ctxvault": analysis_server
            }
        });

        let scout_path = agents_dir.join("ctxvault-scout.json");
        let analysis_path = agents_dir.join("ctxvault-analysis.json");

        if dry_run {
            summary.dry_run_detected.push(format!(
                "Kiro Subagents -> Would write scout & analysis profiles to: {}",
                agents_dir.display()
            ));
        } else {
            let _ = fs::create_dir_all(&agents_dir);
            if let Ok(content) = serde_json::to_string_pretty(&scout_profile) {
                let _ = fs::write(&scout_path, content);
            }
            if let Ok(content) = serde_json::to_string_pretty(&analysis_profile) {
                let _ = fs::write(&analysis_path, content);
            }
            summary.configured.push(format!("Kiro Subagent Profiles [{}]", agents_dir.display()));
        }
    }

    if let Some(ref reg) = clients_registry {
        if !dry_run {
            let mut global_cfg = ctxvault_common::config::load_global_config();
            global_cfg.auth = reg.clone();
            let _ = ctxvault_common::config::save_global_config(&global_cfg);
            let central_path = ctxvault_common::client::get_central_clients_path();
            let _ = ctxvault_common::client::save_clients_config(reg, &central_path);
        }
    }

    Ok(summary)
}

/// Auto-populate credentials into detected MCP client configuration files.
pub fn autopopulate_clients(
    install_dir: Option<&Path>,
    dry_run: bool,
    filter_agents: Option<&[String]>,
) -> anyhow::Result<InstallSummary> {
    run_install(install_dir, dry_run, true, false, None, filter_agents, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_detect_agents_includes_kiro() {
        let agents = detect_agents();
        assert!(
            agents.iter().any(|a| a.name.contains("Kiro")),
            "Expected Kiro CLI in detected agents: {:?}",
            agents.iter().map(|a| a.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_kiro_installer_auto_configuration() {
        let tmp = TempDir::new().unwrap();
        let ws = tmp.path().join("workspace");
        fs::create_dir_all(&ws).unwrap();

        // Create workspace .kiro directory to simulate an active Kiro workspace
        let ws_kiro = ws.join(".kiro");
        let ws_kiro_settings = ws_kiro.join("settings");
        fs::create_dir_all(&ws_kiro_settings).unwrap();

        let summary = run_install(None, false, true, true, Some(&ws), None, false).unwrap();

        // 1. Verify workspace Kiro mcp.json was created/configured
        let mcp_json_path = ws_kiro_settings.join("mcp.json");
        assert!(mcp_json_path.exists(), "mcp.json should be written");
        let mcp_content: Value =
            serde_json::from_str(&fs::read_to_string(&mcp_json_path).unwrap()).unwrap();
        assert!(
            mcp_content["mcpServers"]["ctxvault"]["command"].is_string(),
            "ctxvault server command should be configured in mcpServers"
        );

        // 2. Verify Kiro subagent profiles were generated
        let scout_agent_path = ws_kiro.join("agents").join("ctxvault-scout.json");
        let analysis_agent_path = ws_kiro.join("agents").join("ctxvault-analysis.json");
        assert!(scout_agent_path.exists(), "ctxvault-scout.json should be created");
        assert!(analysis_agent_path.exists(), "ctxvault-analysis.json should be created");

        let scout_agent: Value =
            serde_json::from_str(&fs::read_to_string(&scout_agent_path).unwrap()).unwrap();
        assert_eq!(scout_agent["name"], "ctxvault-scout");
        let scout_args = scout_agent["mcpServers"]["ctxvault"]["args"].as_array().unwrap();
        assert_eq!(scout_args, &vec![json!("--profile"), json!("scout")]);

        // 3. Verify Kiro steering rule was installed
        let rules_path = ws_kiro.join("rules").join("ctxvault.md");
        let steering_path = ws_kiro.join("steering").join("ctxvault.md");
        assert!(rules_path.exists(), "rules/ctxvault.md should be written");
        assert!(steering_path.exists(), "steering/ctxvault.md should be written");
        assert!(fs::read_to_string(&rules_path)
            .unwrap()
            .contains("ctxvault MCP Steering Protocol"));

        assert!(!summary.configured.is_empty());
    }

    #[test]
    fn test_installer_with_auth_populates_credentials() {
        let tmp = TempDir::new().unwrap();
        let ws = tmp.path().join("workspace_auth");
        let ws_kiro_settings = ws.join(".kiro").join("settings");
        fs::create_dir_all(&ws_kiro_settings).unwrap();

        let summary = run_install(None, false, true, false, Some(&ws), None, true).unwrap();
        assert!(!summary.configured.is_empty());

        let mcp_json_path = ws_kiro_settings.join("mcp.json");
        assert!(mcp_json_path.exists());
        let mcp_content: Value =
            serde_json::from_str(&fs::read_to_string(&mcp_json_path).unwrap()).unwrap();

        let env_key = mcp_content["mcpServers"]["ctxvault"]["env"]["CTXV_API_KEY"].as_str();
        assert!(env_key.is_some(), "CTXV_API_KEY must be populated in mcpServers.ctxvault.env");
        assert!(env_key.unwrap().starts_with("kiro_"));
    }
}
