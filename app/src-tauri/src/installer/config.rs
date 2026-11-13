use super::{
    files::{Tree, file_tree, identifier, text},
    types::*,
};
use serde_json::{Value, json};
use toml_edit::{DocumentMut, Item, Table};
pub fn semantic_hash(value: &Option<Value>) -> String {
    hash(serde_json::to_string(value).unwrap_or_default().as_bytes())
}
pub fn entry(tree: &Tree, format: &str, section: &str, key: &str) -> Result<Option<Value>> {
    let raw = text(tree)?;
    if raw.trim().is_empty() {
        return Ok(None);
    }
    if format == "toml" {
        let doc: toml::Value = toml::from_str(&raw).map_err(|_| {
            fail(
                "CONFIG_PARSE",
                "The existing TOML is invalid; it was not changed.",
            )
        })?;
        Ok(doc
            .get(section)
            .and_then(|s| s.get(key))
            .and_then(|v| serde_json::to_value(v).ok()))
    } else {
        let doc: Value = serde_json::from_str(&raw).map_err(|_| {
            fail(
                "MARKETPLACE_PARSE",
                "The existing marketplace JSON is invalid.",
            )
        })?;
        if doc["name"].as_str() != Some(section) {
            return Err(fail(
                "MARKETPLACE_CONFLICT",
                "This marketplace belongs to another owner/name. It will not be overwritten.",
            ));
        }
        let plugins = doc["plugins"]
            .as_array()
            .ok_or_else(|| fail("MARKETPLACE_PARSE", "Marketplace plugins must be an array."))?;
        let matching = plugins
            .iter()
            .filter(|v| v["name"] == key)
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            return Err(fail(
                "MARKETPLACE_CONFLICT",
                "Duplicate marketplace names require manual repair.",
            ));
        }
        Ok(matching.first().map(|v| (*v).clone()))
    }
}
pub fn apply(tree: &Tree, patch: &Patch) -> Result<Tree> {
    if semantic_hash(&entry(tree, &patch.format, &patch.section, &patch.key)?)
        != patch.expected_hash
    {
        return Err(fail(
            "CONFIG_CONFLICT",
            "The managed configuration entry changed after preview.",
        ));
    }
    let raw = text(tree)?;
    let output = if patch.format == "toml" {
        let mut doc: DocumentMut = raw
            .parse()
            .map_err(|_| fail("CONFIG_PARSE", "Existing TOML cannot be safely edited."))?;
        if let Some(value) = patch.value.as_ref() {
            let rendered = toml_edit::ser::to_document(value)
                .map_err(|_| fail("CONFIG", "The requested entry is not supported TOML."))?;
            if doc.get(&patch.section).is_none() {
                let mut table = Table::new();
                table.set_implicit(true);
                doc[&patch.section] = Item::Table(table);
            }
            let table = doc[&patch.section].as_table_like_mut().ok_or_else(|| {
                fail(
                    "CONFIG_CONFLICT",
                    "The existing configuration section is not a table.",
                )
            })?;
            table.insert(&patch.key, Item::Table(rendered.into_table()));
        } else if let Some(table) = doc
            .get_mut(&patch.section)
            .and_then(Item::as_table_like_mut)
        {
            table.remove(&patch.key);
        }
        doc.to_string()
    } else {
        let mut doc: Value = if raw.trim().is_empty() {
            json!({"name":patch.section,"interface":{"displayName":"SkillVerse managed local packages"},"plugins":[]})
        } else {
            serde_json::from_str(&raw)
                .map_err(|_| fail("MARKETPLACE_PARSE", "Invalid marketplace JSON."))?
        };
        if doc["name"] != patch.section {
            return Err(fail(
                "MARKETPLACE_CONFLICT",
                "An unrelated marketplace will not be replaced.",
            ));
        }
        let array = doc["plugins"].as_array_mut().ok_or_else(|| {
            fail(
                "MARKETPLACE_PARSE",
                "Marketplace plugin entries are invalid.",
            )
        })?;
        array.retain(|entry| entry["name"] != patch.key);
        if let Some(value) = patch.value.as_ref() {
            array.push(value.clone());
        }
        serde_json::to_string_pretty(&doc)
            .map_err(|_| fail("CONFIG", "Marketplace encoding failed."))?
            + "\n"
    };
    // Reparse before it can ever reach the executor.
    let result = file_tree(output.as_bytes());
    let actual = entry(&result, &patch.format, &patch.section, &patch.key)?;
    if actual != patch.value {
        return Err(fail("CONFIG", "Configuration merge verification failed."));
    }
    Ok(result)
}
pub fn new_patch(
    tree: &Tree,
    format: &str,
    section: &str,
    key: &str,
    value: Option<Value>,
) -> Result<Patch> {
    let expected_hash = semantic_hash(&entry(tree, format, section, key)?);
    let patch = Patch {
        format: format.into(),
        section: section.into(),
        key: key.into(),
        expected_hash,
        value,
    };
    apply(tree, &patch)?;
    Ok(patch)
}
pub fn mcp_value(draft: &McpDraft) -> Result<Value> {
    identifier(&draft.name)?;
    if draft.args.len() > 64 || draft.environment_names.len() > 40 {
        return Err(fail(
            "CONFIG_LIMIT",
            "The MCP declaration exceeds its bounds.",
        ));
    }
    for key in draft
        .environment_names
        .iter()
        .chain(draft.bearer_token_environment.iter())
    {
        if key.is_empty()
            || key.len() > 100
            || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(fail(
                "ENVIRONMENT",
                "Use environment variable names, never literal credentials.",
            ));
        }
    }
    if draft.args.iter().any(|v| {
        v.len() > 1000
            || v.contains(['\0', '\r', '\n'])
            || [
                "--token",
                "--password",
                "--api-key",
                "ghp_",
                "github_pat_",
                "bearer ",
            ]
            .iter()
            .any(|marker| v.to_lowercase().contains(marker))
    }) {
        return Err(fail(
            "CREDENTIALS",
            "Sensitive arguments must use environment variable references, not plaintext values.",
        ));
    }
    let mut value = json!({"enabled":false});
    match draft.transport.as_str() {
        "stdio" => {
            let command = draft
                .command
                .as_deref()
                .ok_or_else(|| fail("COMMAND", "An MCP stdio command is required."))?;
            if command.is_empty()
                || command.len() > 300
                || command.contains(['\r', '\n', '\0', '|', ';'])
            {
                return Err(fail(
                    "COMMAND",
                    "Use an executable plus a separate argument list; no shell expression is accepted.",
                ));
            }
            value["command"] = json!(command);
            value["args"] = json!(draft.args);
            value["env_vars"] = json!(draft.environment_names);
        }
        "http" => {
            let url = draft
                .url
                .as_deref()
                .ok_or_else(|| fail("URL", "An MCP URL is required."))?;
            let parsed =
                reqwest::Url::parse(url).map_err(|_| fail("URL", "The MCP URL is invalid."))?;
            let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
            if !(parsed.scheme() == "https" || (parsed.scheme() == "http" && local))
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
            {
                return Err(fail(
                    "URL",
                    "Use HTTPS (or loopback HTTP), without embedded credentials or query secrets.",
                ));
            }
            value["url"] = json!(url);
            if let Some(variable) = draft.bearer_token_environment.as_ref() {
                value["bearer_token_env_var"] = json!(variable);
            }
        }
        _ => {
            return Err(fail(
                "TRANSPORT",
                "Only stdio and Streamable HTTP declarations are supported.",
            ));
        }
    }
    if !crate::github::analysis::risks(&value.to_string(), &[]).is_empty() {
        return Err(fail(
            "REVIEW_REQUIRED",
            "The MCP command/configuration contains a static security finding.",
        ));
    }
    Ok(value)
}
