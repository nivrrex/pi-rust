//! `--list-models [search]`：列出可用模型（ref `cli/list-models.ts:1-115`）。
//!
//! 输出格式与上游逐字一致：6 列、两空格分隔、按 provider 再按 model id 排序，
//! token 数按 `formatTokenCount`（`200000 → 200K`、`1000000 → 1M`、`1500000 → 1.5M`）。
//! 数据源是 `models.json` 与内置目录的并集（内置目录兜底，见 `runtime::load_builtin_catalog`）。

use pi_ai::config::{BuiltInCatalog, ModelsConfig, ProviderConfig};

/// 一行模型信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRow {
    pub provider: String,
    pub id: String,
    pub context_window: Option<u64>,
    pub max_tokens: Option<u64>,
    pub reasoning: bool,
    pub images: bool,
}

/// token 数的人类可读写法（ref `list-models.ts:16-28` 的 `formatTokenCount`）。
#[must_use]
pub fn format_token_count(count: u64) -> String {
    if count >= 1_000_000 {
        let millions = count as f64 / 1_000_000.0;
        if millions.fract() == 0.0 {
            format!("{}M", millions as u64)
        } else {
            format!("{millions:.1}M")
        }
    } else if count >= 1_000 {
        let thousands = count as f64 / 1_000.0;
        if thousands.fract() == 0.0 {
            format!("{}K", thousands as u64)
        } else {
            format!("{thousands:.1}K")
        }
    } else {
        count.to_string()
    }
}

/// 从 `models.json` 文本与内置目录收集**可用**模型（同 provider+id 时 `models.json` 优先）。
///
/// 内置目录只为**已登录/已配置**的 provider 兜底（ref 的 `getAvailable` 只返回已认证 provider
/// 的模型）：`authenticated` 来自 `auth.json` 的键。没有 models.json、也没有登录时结果为空，
/// 与上游一样输出 "No models available"。
#[must_use]
pub fn collect_models(
    models_json: Option<&str>,
    catalog: Option<&BuiltInCatalog>,
    authenticated: &std::collections::BTreeSet<String>,
) -> Vec<ModelRow> {
    let mut rows: Vec<ModelRow> = Vec::new();
    let mut seen: std::collections::BTreeSet<(String, String)> = std::collections::BTreeSet::new();

    if let Some(text) = models_json {
        if let Ok(config) = serde_json::from_str::<ModelsConfig>(text) {
            for (provider_name, raw) in &config.providers {
                let Ok(provider) = serde_json::from_value::<ProviderConfig>(raw.clone()) else {
                    continue;
                };
                for model in &provider.models {
                    let key = (provider_name.clone(), model.id.clone());
                    if !seen.insert(key) {
                        continue;
                    }
                    rows.push(ModelRow {
                        provider: provider_name.clone(),
                        id: model.id.clone(),
                        context_window: model.context_window,
                        max_tokens: model.max_tokens,
                        reasoning: model.reasoning.unwrap_or(false),
                        images: model
                            .input
                            .as_ref()
                            .is_some_and(|input| input.iter().any(|kind| kind == "image")),
                    });
                }
            }
        }
    }

    if let Some(catalog) = catalog {
        for (provider, model) in catalog.entries() {
            if !authenticated.contains(provider) {
                continue;
            }
            let key = (provider.to_string(), model.id.clone());
            if !seen.insert(key) {
                continue;
            }
            rows.push(ModelRow {
                provider: provider.to_string(),
                id: model.id.clone(),
                context_window: model.context_window,
                max_tokens: model.max_tokens,
                // 内置目录不带 `reasoning`/`input` 字段（导出时被裁掉了），按上游缺省处理。
                reasoning: false,
                images: false,
            });
        }
    }

    rows
}

/// 可用 provider 集合：`auth.json` 里有凭据的、`models.json` 里配了 `apiKey` 的、
/// 或对应环境变量已设置的（ref 的 `getAvailable` 只返回已认证 provider 的模型）。
#[must_use]
pub fn authenticated_providers(
    agent_dir: &std::path::Path,
    models_json: Option<&str>,
) -> std::collections::BTreeSet<String> {
    let mut names: std::collections::BTreeSet<String> =
        std::fs::read_to_string(agent_dir.join("auth.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|value| value.as_object().cloned())
            .map(|object| object.keys().cloned().collect())
            .unwrap_or_default();
    if let Some(text) = models_json {
        if let Ok(config) = serde_json::from_str::<ModelsConfig>(text) {
            for (name, raw) in &config.providers {
                let has_key = raw
                    .get("apiKey")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|key| !key.trim().is_empty());
                let api = raw
                    .get("api")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let api_env = if api == "anthropic-messages" {
                    Some("ANTHROPIC_API_KEY")
                } else if api == "openai-completions" || api == "openai-responses" {
                    Some("OPENAI_API_KEY")
                } else {
                    None
                };
                if has_key || api_env.is_some_and(|key| std::env::var(key).is_ok()) {
                    names.insert(name.clone());
                }
            }
        }
    }
    names
}

/// 某个 `(provider, model)` 是否推理模型（`Ctrl+P` 切换后按它决定思考档位与提示文案）。
#[must_use]
pub fn model_is_reasoning(agent_dir: &std::path::Path, provider: &str, id: &str) -> bool {
    let models_json = std::fs::read_to_string(agent_dir.join("models.json")).ok();
    let catalog = crate::runtime::load_builtin_catalog(agent_dir);
    let authenticated = authenticated_providers(agent_dir, models_json.as_deref());
    collect_models(models_json.as_deref(), catalog.as_ref(), &authenticated)
        .iter()
        .find(|row| row.provider == provider && row.id == id)
        .is_some_and(|row| row.reasoning)
}

/// 把行列表转成 TUI 的 `/model` 候选（`Ctrl+P` 循环与 `/model` 面板同源）。
#[must_use]
pub fn to_model_choices(rows: &[ModelRow]) -> Vec<crate::tui::ModelChoice> {
    rows.iter()
        .map(|row| crate::tui::ModelChoice {
            provider: row.provider.clone(),
            id: row.id.clone(),
        })
        .collect()
}

/// 模糊过滤（ref 用 `fuzzyFilter(models, pattern, m => `${m.provider} ${m.id}`)`）：
/// 这里用**子序列**匹配（大小写不敏感），与上游的模糊语义同量级，不做排序重排。
#[must_use]
pub fn filter_models(rows: Vec<ModelRow>, pattern: &str) -> Vec<ModelRow> {
    let needle: Vec<char> = pattern.to_lowercase().chars().collect();
    if needle.is_empty() {
        return rows;
    }
    rows.into_iter()
        .filter(|row| {
            let haystack = format!("{} {}", row.provider, row.id).to_lowercase();
            let mut iter = haystack.chars();
            needle.iter().all(|want| iter.any(|ch| ch == *want))
        })
        .collect()
}

/// 渲染成上游那份表格（`provider  model  context  max-out  thinking  images`）。
#[must_use]
pub fn render_table(rows: &[ModelRow]) -> String {
    let mut rows: Vec<&ModelRow> = rows.iter().collect();
    rows.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| left.id.cmp(&right.id))
    });
    let cells: Vec<[String; 6]> = rows
        .iter()
        .map(|row| {
            [
                row.provider.clone(),
                row.id.clone(),
                row.context_window
                    .map(format_token_count)
                    .unwrap_or_else(|| "-".to_string()),
                row.max_tokens
                    .map(format_token_count)
                    .unwrap_or_else(|| "-".to_string()),
                if row.reasoning { "yes" } else { "no" }.to_string(),
                if row.images { "yes" } else { "no" }.to_string(),
            ]
        })
        .collect();
    let headers = [
        "provider", "model", "context", "max-out", "thinking", "images",
    ];
    let mut widths = [0usize; 6];
    for (index, header) in headers.iter().enumerate() {
        widths[index] = cells
            .iter()
            .map(|row| row[index].chars().count())
            .max()
            .unwrap_or(0)
            .max(header.chars().count());
    }
    let line = |row: &[String]| {
        row.iter()
            .enumerate()
            .map(|(index, cell)| {
                let pad = widths[index].saturating_sub(cell.chars().count());
                format!("{cell}{}", " ".repeat(pad))
            })
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    let header: Vec<String> = headers.iter().map(|h| (*h).to_string()).collect();
    let mut out = line(&header);
    for row in &cells {
        out.push('\n');
        out.push_str(&line(row));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{collect_models, filter_models, format_token_count, render_table, ModelRow};

    #[test]
    fn token_counts_match_upstream_format() {
        assert_eq!(format_token_count(128_000), "128K");
        assert_eq!(format_token_count(200_000), "200K");
        assert_eq!(format_token_count(1_000_000), "1M");
        assert_eq!(format_token_count(1_500_000), "1.5M");
        assert_eq!(format_token_count(512), "512");
    }

    #[test]
    fn table_header_and_alignment_match_upstream_shape() {
        let rows = vec![
            ModelRow {
                provider: "openrouter".to_string(),
                id: "free".to_string(),
                context_window: Some(200_000),
                max_tokens: Some(8_192),
                reasoning: true,
                images: true,
            },
            ModelRow {
                provider: "deepseek".to_string(),
                id: "deepseek-v4.1-flash".to_string(),
                context_window: Some(128_000),
                max_tokens: None,
                reasoning: false,
                images: false,
            },
        ];
        let text = render_table(&rows);
        let lines: Vec<&str> = text.lines().collect();
        let header = lines[0];
        for column in [
            "provider", "model", "context", "max-out", "thinking", "images",
        ] {
            assert!(header.contains(column), "表头缺 {column}：{header:?}");
        }
        // 按 provider 再按 id 排序：deepseek 在前；每行 6 列、两空格分隔。
        assert!(lines[1].starts_with("deepseek"), "{:?}", lines[1]);
        assert!(lines[1].contains("deepseek-v4.1-flash"));
        assert!(lines[1].contains("128K"));
        assert!(lines[1].trim_end().ends_with("no"), "{:?}", lines[1]);
        assert!(lines[2].starts_with("openrouter"));
    }

    #[test]
    fn models_json_wins_over_the_builtin_catalog() {
        let json = r#"{"providers":{"p":{"models":[{"id":"m","contextWindow":1000,"maxTokens":10,"reasoning":true}]}}}"#;
        let catalog = pi_ai::config::BuiltInCatalog::from_json(
            r#"{"p":[{"id":"m","api":"openai-completions"},{"id":"other","api":"openai-completions"}]}"#,
        )
        .expect("目录");
        let mut auth = std::collections::BTreeSet::new();
        auth.insert("p".to_string());
        let rows = collect_models(Some(json), Some(&catalog), &auth);
        assert_eq!(rows.len(), 2, "同 id 只留 models.json 的那条：{rows:?}");
        let first = rows.iter().find(|r| r.id == "m").expect("m");
        assert_eq!(first.context_window, Some(1000));
        assert!(first.reasoning);
    }

    #[test]
    fn fuzzy_filter_matches_subsequences() {
        let rows = vec![
            ModelRow {
                provider: "openrouter".to_string(),
                id: "deepseek-v4.1-flash".to_string(),
                context_window: None,
                max_tokens: None,
                reasoning: false,
                images: false,
            },
            ModelRow {
                provider: "anthropic".to_string(),
                id: "claude".to_string(),
                context_window: None,
                max_tokens: None,
                reasoning: false,
                images: false,
            },
        ];
        let hit = filter_models(rows.clone(), "dsv4");
        assert_eq!(hit.len(), 1, "应命中 deepseek-v4：{hit:?}");
        assert_eq!(hit[0].id, "deepseek-v4.1-flash");
        assert!(filter_models(rows, "zzz").is_empty());
    }
}
