use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
pub type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub capability_id: Option<String>,
    pub capability_type: String,
    pub capability_version: Option<String>,
    pub description: String,
    pub dependencies: Vec<String>,
    pub input_schema: Value,
    pub output_schema: Value,
    pub adapter: String,
    pub risk: String,
    pub estimated_cost: String,
    pub timeout_ms: u64,
    pub max_attempts: u32,
    pub retry_policy: String,
    pub verification_policy: String,
    pub params: Value,
    pub condition: Option<Condition>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Condition {
    pub node: String,
    pub field: String,
    pub equals: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Plan {
    pub workflow_id: String,
    pub revision: i64,
    pub goal: String,
    pub project_root: String,
    pub complexity: String,
    pub scope: String,
    pub goal_verifiable: bool,
    pub nodes: Vec<Node>,
    pub routing: Value,
}
impl Plan {
    pub fn digest(&self) -> String {
        crate::github::types::digest(serde_json::to_string(self).unwrap().as_bytes())
    }
    pub fn validate(&self) -> Result<Vec<String>> {
        if self.nodes.is_empty()
            || self.nodes.len() > 32
            || self.goal.trim().is_empty()
            || self.goal.len() > 2000
            || serde_json::to_vec(self).unwrap().len() > 128000
        {
            return Err("INVALID_PLAN_SIZE".into());
        }
        if self.routing["projectRouting"]["executionProfile"] == "FIRST_PARTY_CODEX_READ" {
            let expected = BTreeSet::from([
                "builtin.inspect",
                "builtin.route",
                "builtin.report",
                "builtin.verify",
                "codex.fixture_read",
            ]);
            let actual = self
                .nodes
                .iter()
                .map(|node| node.adapter.as_str())
                .collect::<BTreeSet<_>>();
            if self.nodes.len() != expected.len()
                || actual != expected
                || self
                    .nodes
                    .iter()
                    .any(|node| node.condition.is_some() || node.capability_id.is_some())
            {
                return Err("FIRST_PARTY_EXECUTION_PROFILE_CHANGED".into());
            }
            let probe = self
                .nodes
                .iter()
                .find(|node| node.adapter == "codex.fixture_read")
                .unwrap();
            let report = self
                .nodes
                .iter()
                .find(|node| node.adapter == "builtin.report")
                .unwrap();
            let verify = self
                .nodes
                .iter()
                .find(|node| node.adapter == "builtin.verify")
                .unwrap();
            if !report.dependencies.contains(&probe.id) || !verify.dependencies.contains(&report.id)
            {
                return Err("FIRST_PARTY_EVIDENCE_CHAIN_CHANGED".into());
            }
        }
        for unique in [
            "builtin.report",
            "codex.fixture_read",
            "codex.skill_fixture",
            "mcp.fixture",
        ] {
            if self.nodes.iter().filter(|n| n.adapter == unique).count() > 1 {
                return Err("DUPLICATE_EFFECT_ADAPTER".into());
            }
        }
        let mut ids = BTreeSet::new();
        for n in &self.nodes {
            if n.id == "mission-core"
                || n.id.is_empty()
                || n.id.len() > 64
                || !n
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                || !ids.insert(n.id.clone())
                || n.description.trim().is_empty()
                || n.description.len() > 1000
                || !(100..=120000).contains(&n.timeout_ms)
                || !(1..=3).contains(&n.max_attempts)
            {
                return Err("INVALID_OR_DUPLICATE_NODE".into());
            }
            super::adapters::validate(n)?;
        }
        let mut degree: BTreeMap<String, usize> = self
            .nodes
            .iter()
            .map(|n| (n.id.clone(), n.dependencies.len()))
            .collect();
        for n in &self.nodes {
            let mut seen = BTreeSet::new();
            for d in &n.dependencies {
                if !ids.contains(d) || d == &n.id || !seen.insert(d) {
                    return Err("INVALID_OR_DUPLICATE_EDGE".into());
                }
            }
            if let Some(c) = &n.condition {
                if !n.dependencies.contains(&c.node)
                    || !((c.field == "externalReady" && c.equals.is_boolean())
                        || (c.field == "status"
                            && matches!(
                                c.equals.as_str(),
                                Some("SUCCEEDED" | "FAILED" | "BLOCKED")
                            )))
                {
                    return Err("INVALID_CONDITION".into());
                }
            }
        }
        let mut order = Vec::new();
        loop {
            let ready: Vec<_> = degree
                .iter()
                .filter(|(_, v)| **v == 0)
                .map(|(id, _)| id.clone())
                .collect();
            if ready.is_empty() {
                break;
            }
            for id in ready {
                degree.remove(&id);
                order.push(id.clone());
                for n in &self.nodes {
                    if n.dependencies.contains(&id) {
                        if let Some(v) = degree.get_mut(&n.id) {
                            *v -= 1
                        }
                    }
                }
            }
        }
        if !degree.is_empty() {
            return Err("CYCLIC_DEPENDENCY".into());
        }
        Ok(order)
    }
}
pub fn base_node(id: &str, adapter: &str, description: &str, deps: &[&str]) -> Node {
    Node {
        id: id.into(),
        capability_id: None,
        capability_type: "builtin".into(),
        capability_version: Some("skillverse-v1".into()),
        description: description.into(),
        dependencies: deps.iter().map(|d| d.to_string()).collect(),
        input_schema: json!({"type":"object"}),
        output_schema: json!({"type":"object"}),
        adapter: adapter.into(),
        risk: if adapter == "builtin.report" {
            "MEDIUM"
        } else {
            "LOW"
        }
        .into(),
        estimated_cost: "No model/API charge; bounded local execution".into(),
        timeout_ms: 15000,
        max_attempts: 1,
        retry_policy: "SAFE_READ_ONLY".into(),
        verification_policy: "HASH_BOUND_FIRST_PARTY_OUTPUT_V1".into(),
        params: json!({}),
        condition: None,
    }
}
