use super::{types::*, *};
use crate::intelligence::Subject;
fn family(s: &Subject) -> String {
    if let Some(c) = &s.category {
        return c.clone();
    }
    let text = format!("{} {}", s.name, s.summary).to_lowercase();
    for (name, terms) in [
        ("browser", &["browser", "playwright"][..]),
        ("frontend", &["frontend", "interface", "react"][..]),
        ("documents", &["document", "pdf", "word"][..]),
        ("analytics", &["analytics", "spreadsheet", "csv"][..]),
        ("research", &["github", "research"][..]),
    ] {
        if terms.iter().any(|term| text.contains(term)) {
            return name.into();
        }
    }
    "general".into()
}
pub fn route(goal: &str, subjects: &[Subject], preferred: Option<&str>) -> Value {
    let query = goal.to_lowercase();
    if [
        "inspect project metadata",
        "生成项目元数据报告",
        "capability inventory",
        "readiness report",
    ]
    .contains(&query.trim())
    {
        return json!({"intentCategories":["metadata"],"recommendations":[],"externalReady":false,"selectionRule":"The explicit metadata/readiness task is handled by deterministic built-ins.","simplerAlternative":"No external Skill, Plugin, MCP or model is necessary.","omitted":"External capabilities would add setup and risk without helping this bounded task.","liveModel":"NOT VERIFIED"});
    }
    let intents = [
        (
            "browser",
            ["ui", "网页", "网站", "react", "test", "测试", "browser"].as_slice(),
        ),
        (
            "documents",
            ["word", "pdf", "文档", "document", "整理"].as_slice(),
        ),
        (
            "code",
            ["代码", "code", "项目", "project", "后端"].as_slice(),
        ),
        (
            "research",
            ["github", "issue", "仓库", "research"].as_slice(),
        ),
        (
            "analytics",
            ["数据", "csv", "spreadsheet", "统计", "data"].as_slice(),
        ),
    ];
    let categories: Vec<_> = intents
        .iter()
        .filter(|(_, words)| words.iter().any(|w| query.contains(w)))
        .map(|(c, _)| *c)
        .collect();
    let mut ranked: Vec<_> = subjects
        .iter()
        .map(|s| {
            let text = format!(
                "{} {} {}",
                s.name,
                s.summary,
                s.category.as_deref().unwrap_or("")
            )
            .to_lowercase();
            let mut score = categories
                .iter()
                .filter(|c| match **c {
                    "browser" => ["browser", "playwright", "frontend", "interface"]
                        .iter()
                        .any(|w| text.contains(w)),
                    "documents" => ["document", "pdf", "word"].iter().any(|w| text.contains(w)),
                    "research" => ["research", "github", "issue"]
                        .iter()
                        .any(|w| text.contains(w)),
                    "analytics" => ["analytics", "spreadsheet", "csv", "data analysis"]
                        .iter()
                        .any(|w| text.contains(w)),
                    _ => text.contains(**c),
                })
                .count()
                * 10;
            for word in query
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.len() > 2)
            {
                if text.contains(word) {
                    score += 2
                }
            }
            if s.installed {
                score += 2
            }
            if preferred == Some(s.logical_id.as_str()) {
                score += 100
            }
            (score, s)
        })
        .filter(|(score, _)| *score > 2)
        .collect();
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.logical_id.cmp(&b.1.logical_id))
    });
    let results:Vec<_>=ranked.iter().take(5).enumerate().map(|(index,(score,s))|json!({"role":if index==0{"PRIMARY"}else{"ALTERNATIVE_OR_SUPPORTING"},"function":family(s),"logicalId":s.logical_id,"entityId":s.entity_id,"name":s.name,"kind":s.kind,"version":crate::learning::source_version(s),"sourceEvidence":s.evidence["id"],"observedInstallation":s.observed_status,"filesPresent":s.evidence["document"]["availability"]=="READ_ONLY_SOURCE","recognition":"NOT_TESTED_IN_THIS_PROJECT","runtime":"NOT_VERIFIED","ready":false,"score":score,"confidence":(*score as f64/100.).min(0.85),"reason":format!("Documented purpose matches {:?}; {}. Installation is not runtime readiness.",categories,if s.installed{"saved local observation"}else{"discovery candidate requiring setup"}),"nextStep":match s.kind.as_str(){"skill"=>"Recognize the exact Skill in this project's Codex session, then separately approve inference", "plugin"=>"Inspect actual bundled Skills/MCP and their account/runtime requirements",_=>"Connect only an approved MCP, discover tools and validate its schema before a call"}})).collect();
    json!({"intentCategories":categories,"recommendations":results,"externalReady":false,"selectionRule":"Source/category and goal terms; saved installation is a tie-breaker. Rules confidence is not measured model probability.","simplerAlternative":"Run the built-in project-metadata/readiness report without external models or tools.","omitted":"Unrelated capabilities are not added to inflate the workflow.","liveModel":"NOT VERIFIED"})
}
pub fn generate(
    goal: String,
    project_root: String,
    subjects: &[Subject],
    preferred: Option<&str>,
) -> Result<Plan> {
    let routing = route(&goal, subjects, preferred);
    let complexity = if goal.len() > 140 || goal.contains("然后") || goal.contains("→") {
        "COMPLEX"
    } else if goal.contains("并") || goal.contains(" and ") || goal.contains("测试") {
        "MEDIUM"
    } else {
        "SIMPLE"
    };
    let mut nodes = vec![
        base_node(
            "context",
            "builtin.inspect",
            "Inspect explicitly selected project metadata",
            &[],
        ),
        base_node(
            "routing",
            "builtin.route",
            "Match documented capabilities and explain readiness",
            &[],
        ),
    ];
    let recommendations = routing["recommendations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut chosen = Vec::new();
    for cap in &recommendations {
        if !matches!(cap["kind"].as_str(), Some("skill" | "plugin" | "mcp")) {
            continue;
        }
        if chosen.is_empty() {
            chosen.push(cap)
        } else if (complexity != "SIMPLE" || goal.to_lowercase().contains("ui"))
            && chosen[0]["function"] != cap["function"]
        {
            chosen.push(cap);
            break;
        }
    }
    for (index, cap) in chosen.into_iter().enumerate() {
        let kind = cap["kind"].as_str().ok_or("CAPABILITY_TYPE")?;
        let adapter = match kind {
            "skill" => "codex.skill",
            "plugin" => "plugin.bundle",
            "mcp" => "mcp.tool",
            _ => return Err("UNSUPPORTED_CAPABILITY_TYPE".into()),
        };
        let id = if index == 0 {
            "capability"
        } else {
            "supporting-capability"
        };
        let mut n = base_node(
            id,
            adapter,
            &format!(
                "{}: {}",
                if index == 0 {
                    "Primary capability"
                } else {
                    "Complementary capability"
                },
                cap["name"].as_str().unwrap_or("Unknown")
            ),
            &["routing"],
        );
        n.capability_id = cap["logicalId"].as_str().map(String::from);
        n.capability_version = cap["version"].as_str().map(String::from);
        n.capability_type = kind.into();
        n.condition = Some(Condition {
            node: "routing".into(),
            field: "externalReady".into(),
            equals: json!(true),
        });
        nodes.push(n);
    }
    let mut report = base_node(
        "report",
        "builtin.report",
        "Write a scoped evidence report in the approved application workspace",
        &["context", "routing"],
    );
    report.retry_policy = "OWNED_IDEMPOTENT".into();
    nodes.push(report);
    nodes.push(base_node(
        "verify",
        "builtin.verify",
        "Re-read the report and verify its content digest",
        &["report"],
    ));
    let lower = goal.to_lowercase();
    let goal_verifiable = [
        "inspect project metadata",
        "生成项目元数据报告",
        "capability inventory",
        "readiness report",
    ]
    .contains(&lower.trim());
    let plan=Plan{workflow_id:uuid::Uuid::new_v4().to_string(),revision:1,goal,project_root,complexity:complexity.into(),scope:"Built-in metadata inspection, capability routing and hash-verified local report; browser behavior, external tool execution and full user-goal success are separate claims.".into(),goal_verifiable,nodes,routing};
    plan.validate()?;
    Ok(plan)
}
