use super::*;
use crate::intelligence::Provider;

pub const SCHEMA_VERSION: u8 = 1;
pub(super) const MAX_PICKS: usize = 3;
pub(super) const MAX_RANKED: usize = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Intent {
    Metadata,
    Frontend,
    BrowserTesting,
    Documents,
    #[serde(rename = "github")]
    GitHub,
    Data,
    Backend,
    Unclear,
}

impl Intent {
    pub fn key(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::Frontend => "frontend",
            Self::BrowserTesting => "browser_testing",
            Self::Documents => "documents",
            Self::GitHub => "github",
            Self::Data => "data",
            Self::Backend => "backend",
            Self::Unclear => "unclear",
        }
    }
    fn categories(self) -> &'static [&'static str] {
        match self {
            Self::Frontend => &["frontend"],
            Self::BrowserTesting => &["browser", "testing"],
            Self::Documents => &["documents", "pdf"],
            Self::GitHub => &["development"],
            Self::Data => &["analytics", "spreadsheets"],
            Self::Backend => &["backend", "databases"],
            _ => &[],
        }
    }
    fn signals(self) -> &'static [&'static str] {
        match self {
            Self::Metadata => &[
                "inspect project metadata",
                "项目元数据",
                "capability inventory",
                "readiness report",
                "技术栈",
            ],
            Self::Frontend => &[
                "frontend",
                "ui",
                "react",
                "vite",
                "vue",
                "website",
                "css",
                "web design",
                "前端",
                "界面",
                "网页",
                "网站",
            ],
            Self::BrowserTesting => &[
                "browser",
                "playwright",
                "selenium",
                "e2e",
                "visual regression",
                "test",
                "测试",
                "浏览器",
                "验收",
            ],
            Self::Documents => &[
                "pdf",
                "docx",
                "word",
                "document",
                "markitdown",
                "文档",
                "提取文字",
            ],
            Self::GitHub => &["github", "issues", "pull request", "仓库", "议题"],
            Self::Data => &[
                "csv",
                "excel",
                "xlsx",
                "spreadsheet",
                "statistics",
                "analysis",
                "表格",
                "数据",
                "统计",
            ],
            Self::Backend => &[
                "backend",
                "fastapi",
                "django",
                "spring boot",
                "api",
                "database",
                "后端",
                "数据库",
                "接口",
            ],
            Self::Unclear => &[],
        }
    }
}

fn has_phrase(text: &str, needle: &str) -> bool {
    if !needle.is_ascii() {
        return text.contains(needle);
    }
    text.match_indices(needle).any(|(start, hit)| {
        let before = text[..start].chars().next_back();
        let after = text[start + hit.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric())
            && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

pub(super) fn intents(goal: &str) -> Vec<Intent> {
    let goal = goal.to_lowercase();
    let mut found: Vec<_> = [
        Intent::Frontend,
        Intent::BrowserTesting,
        Intent::Documents,
        Intent::GitHub,
        Intent::Data,
        Intent::Backend,
    ]
    .into_iter()
    .filter(|kind| kind.signals().iter().any(|term| has_phrase(&goal, term)))
    .collect();
    if found.is_empty() {
        found.push(
            if Intent::Metadata
                .signals()
                .iter()
                .any(|term| has_phrase(&goal, term))
            {
                Intent::Metadata
            } else {
                Intent::Unclear
            },
        );
    }
    found
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Candidate {
    pub id: String,
    pub version: String,
    pub name: String,
    pub kind: String,
    pub category: Option<String>,
    pub secondary_categories: Vec<String>,
    pub classification: String,
    #[serde(skip)]
    pub purpose: String,
    pub installation: String,
    pub recognition: String,
    pub runtime: String,
    pub availability: String,
    pub risk: String,
    pub cost: String,
    pub prior_successes: u32,
    pub missing: Vec<String>,
    pub review: Option<ReviewRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct ReviewRef {
    pub id: String,
    pub digest: String,
}

impl Candidate {
    fn reviewable(&self) -> bool {
        self.review.is_some() && self.availability == "SCOPE_REVIEW_AVAILABLE"
    }
    fn unavailable(&self) -> bool {
        matches!(
            self.availability.as_str(),
            "DISABLED" | "UNTRUSTED_METADATA" | "REVIEW_BLOCKED"
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Ranked {
    pub candidate: Candidate,
    pub score: u8,
    pub facets: Vec<Intent>,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Relation {
    pub left: String,
    pub right: String,
    pub kind: String,
    pub state: String,
    pub source: String,
    pub reason: String,
    pub confidence: Option<f64>,
}

fn matches_facet(c: &Candidate, facet: Intent) -> (bool, usize) {
    let kind = c
        .category
        .iter()
        .chain(&c.secondary_categories)
        .any(|key| facet.categories().contains(&key.as_str()));
    let text = c.purpose.to_lowercase();
    let hits = facet
        .signals()
        .iter()
        .filter(|term| has_phrase(&text, term))
        .count();
    (kind, hits)
}

pub(super) fn rank(
    candidates: Vec<Candidate>,
    goals: &[Intent],
    context: &context::Context,
    feedback: &BTreeMap<String, i32>,
) -> Vec<Ranked> {
    let mut ranked = Vec::new();
    for candidate in candidates {
        let scored: Vec<_> = goals
            .iter()
            .copied()
            .filter_map(|facet| {
                let (category, hits) = matches_facet(&candidate, facet);
                (category || hits > 0).then_some((
                    facet,
                    if category { 44 } else { 28 } + (hits.min(4) as i32) * 5,
                ))
            })
            .collect();
        if scored.is_empty() {
            continue;
        }
        let relevance = scored.iter().map(|(_, score)| *score).max().unwrap_or(0);
        let mut points = relevance + (scored.len().saturating_sub(1).min(2) as i32) * 5;
        let mut reasons = vec![format!(
            "Matches {}",
            scored
                .iter()
                .map(|(facet, _)| facet.key())
                .collect::<Vec<_>>()
                .join(", ")
        )];
        if context
            .frameworks
            .iter()
            .any(|f| has_phrase(&candidate.purpose.to_lowercase(), &f.to_lowercase()))
        {
            points += 5;
            reasons.push("Documented purpose overlaps the detected project stack".into());
        }
        if candidate.reviewable() {
            points += 18;
            reasons.push("An immutable reviewed scope can be presented for approval".into());
        } else if candidate.installation == "INSTALLED" {
            points += 4;
            reasons.push("Saved installation observation".into());
        }
        if candidate.prior_successes > 0 {
            points += candidate.prior_successes.min(3) as i32;
            reasons.push("Historical result for this source version".into());
        }
        if candidate.unavailable() {
            points -= 35;
        }
        if candidate.availability == "AUTH_REQUIRED" {
            points -= 8;
        }
        if candidate.risk == "EXTERNAL_SETUP_REQUIRED" {
            points -= 3;
        }
        if let Some(adjustment) = feedback.get(&candidate.id) {
            points += (*adjustment).clamp(-2, 2);
            reasons.push("Small adjustment from repeated feedback for this intent".into());
        }
        ranked.push(Ranked {
            candidate,
            score: points.clamp(0, 100) as u8,
            facets: scored.into_iter().map(|(f, _)| f).collect(),
            reasons,
        });
    }
    ranked.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            // Equal relevance should prefer an existing reviewed scope over a
            // candidate that still needs installation, without claiming readiness.
            .then_with(|| {
                right
                    .candidate
                    .reviewable()
                    .cmp(&left.candidate.reviewable())
            })
            .then_with(|| left.candidate.id.cmp(&right.candidate.id))
    });
    ranked.truncate(MAX_RANKED);
    ranked
}

fn approved_dependency(r: &Relation) -> bool {
    r.state == "APPROVED" && matches!(r.kind.as_str(), "DEPENDS_ON" | "PREREQUISITE")
}
fn incompatible_pair(relations: &[Relation], left: &str, right: &str) -> bool {
    relations.iter().any(|r| {
        r.state == "APPROVED"
            && matches!(r.kind.as_str(), "CONFLICTS_WITH" | "IDENTICAL")
            && ((r.left == left && r.right == right) || (r.left == right && r.right == left))
    })
}

pub(super) fn picks(ranked: &[Ranked], facets: &[Intent], relations: &[Relation]) -> Vec<String> {
    let mut picks = Vec::<String>::new();
    for facet in facets {
        if picks.len() == MAX_PICKS {
            break;
        }
        if picks.iter().any(|id| {
            ranked
                .iter()
                .any(|r| r.candidate.id == *id && r.facets.contains(facet))
        }) {
            continue;
        }
        if let Some(r) = ranked.iter().find(|r| {
            r.facets.contains(facet)
                && !r.candidate.unavailable()
                && !picks
                    .iter()
                    .any(|other| incompatible_pair(relations, other, &r.candidate.id))
        }) {
            picks.push(r.candidate.id.clone());
        }
    }
    picks
}

/// Only referenced review profiles which pass current scope checks can be sent
/// to the existing Mission Control planner. Approved prerequisites go first.
pub(super) fn eligible_reviews(
    ids: &[String],
    ranked: &[Ranked],
    relations: &[Relation],
) -> (Vec<ReviewRef>, Vec<String>) {
    let mut eligible = Vec::new();
    let mut blocked = Vec::new();
    let mut pending: Vec<_> = ids
        .iter()
        .filter_map(|id| {
            ranked
                .iter()
                .find(|r| r.candidate.id == *id && r.candidate.reviewable())
        })
        .collect();
    let mut emitted = BTreeSet::<String>::new();
    loop {
        let Some(index) = pending.iter().position(|candidate| {
            relations
                .iter()
                .filter(|r| r.left == candidate.candidate.id && approved_dependency(r))
                .all(|r| emitted.contains(&r.right))
        }) else {
            break;
        };
        let c = &pending.remove(index).candidate;
        if emitted
            .iter()
            .any(|id| incompatible_pair(relations, id, &c.id))
        {
            continue;
        }
        emitted.insert(c.id.clone());
        eligible.push(c.review.as_ref().unwrap().clone());
    }
    for r in pending {
        blocked.push(r.candidate.id.clone());
    }
    (eligible, blocked)
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum AdviceReason {
    PurposeMatch,
    StackMatch,
    Complementary,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdviceChoice {
    id: String,
    version: String,
    reason: AdviceReason,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Advice {
    schema_version: u8,
    confidence: f64,
    selections: Vec<AdviceChoice>,
}

pub(super) fn validate_advice(
    value: Value,
    ranked: &[Ranked],
    relations: &[Relation],
) -> Result<Advice> {
    if value.to_string().len() > 8192 {
        return Err("SEMANTIC_OUTPUT_TOO_LARGE".into());
    }
    let advice: Advice = serde_json::from_value(value).map_err(|_| "SEMANTIC_SCHEMA_REJECTED")?;
    if advice.schema_version != SCHEMA_VERSION
        || !(0.0..=1.0).contains(&advice.confidence)
        || advice.selections.is_empty()
        || advice.selections.len() > MAX_PICKS
    {
        return Err("SEMANTIC_SCHEMA_REJECTED".into());
    }
    let mut prior = BTreeSet::<String>::new();
    for choice in &advice.selections {
        let candidate = ranked
            .iter()
            .find(|r| r.candidate.id == choice.id && r.candidate.version == choice.version)
            .ok_or("SEMANTIC_UNKNOWN_CAPABILITY_OR_VERSION")?;
        if candidate.candidate.unavailable()
            || prior.contains(&choice.id)
            || prior
                .iter()
                .any(|other| incompatible_pair(relations, other, &choice.id))
            || relations
                .iter()
                .any(|r| r.left == choice.id && approved_dependency(r) && !prior.contains(&r.right))
        {
            return Err("SEMANTIC_UNSAFE_OR_INVALID_ORDER".into());
        }
        prior.insert(choice.id.clone());
    }
    Ok(advice)
}

pub(super) fn checked_selections(
    value: Value,
    ranked: &[Ranked],
    relations: &[Relation],
) -> Result<(Vec<String>, f64)> {
    let advice = validate_advice(value, ranked, relations)?;
    Ok((
        advice.selections.into_iter().map(|s| s.id).collect(),
        advice.confidence,
    ))
}

pub(super) fn model_candidates(ranked: &[Ranked]) -> Vec<Value> {
    ranked.iter().map(|r|json!({"id":r.candidate.id,"version":r.candidate.version,"facets":r.facets,"score":r.score,
        "availability":r.candidate.availability,"risk":r.candidate.risk,"requirements":r.candidate.missing,
        "priorScopedSuccesses":r.candidate.prior_successes.min(100),
        "purpose":if r.candidate.unavailable(){"WITHHELD_UNAVAILABLE_SOURCE".into()}else{clean(&r.candidate.purpose).chars().take(300).collect::<String>()}
    })).collect()
}

pub(super) async fn hybrid(
    provider: Option<&dyn Provider>,
    ranked: &[Ranked],
    goals: &[Intent],
    relations: &[Relation],
) -> (Vec<String>, Value) {
    let fallback = || picks(ranked, goals, relations);
    let Some(provider) = provider else {
        return (
            fallback(),
            json!({"mode":"RULES","status":"LIVE_MODEL_NOT_VERIFIED"}),
        );
    };
    // Bounded metadata projection. Excludes project path, raw goal, full source,
    // credentials, model instructions, and user-provided names.
    let input = json!({"schema_version":SCHEMA_VERSION,"intents":goals,
        "candidates":model_candidates(ranked)});
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        provider.route_advice(&input),
    )
    .await;
    match result {
        Ok(Ok(value)) => match validate_advice(value, ranked, relations) {
            Ok(advice) => (
                advice.selections.iter().map(|r| r.id.clone()).collect(),
                json!({"mode":"OFFLINE_SEMANTIC","status":"VALIDATED_ADVICE","confidence":advice.confidence,
                       "reasons":advice.selections.iter().map(|r|r.reason).collect::<Vec<_>>()}),
            ),
            Err(error) => (fallback(), json!({"mode":"RULES_FALLBACK","status":error})),
        },
        _ => (
            fallback(),
            json!({"mode":"RULES_FALLBACK","status":"SEMANTIC_UNAVAILABLE"}),
        ),
    }
}
