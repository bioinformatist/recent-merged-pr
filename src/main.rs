use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fs;

const DEFAULT_START: &str = "<!-- RECENT_PRS_START -->";
const DEFAULT_END: &str = "<!-- RECENT_PRS_END -->";
const PULL_REQUESTS_QUERY: &str = r#"
query($query: String!, $first: Int!, $after: String) {
  search(query: $query, type: ISSUE, first: $first, after: $after) {
    nodes {
      ... on PullRequest {
        title
        url
        number
        mergedAt
        repository {
          nameWithOwner
          isPrivate
          viewerPermission
        }
      }
    }
    pageInfo {
      hasNextPage
      endCursor
    }
  }
}
"#;
const VIEWER_QUERY: &str = r#"
query {
  viewer {
    login
  }
}
"#;

#[derive(Debug)]
struct Config {
    readme: String,
    marker_start: String,
    marker_end: String,
    include_private: bool,
    repo_limit: usize,
    max_prs_per_repo: usize,
    lookback_days: i64,
    max_pages: usize,
    output_only: bool,
    check: bool,
}

#[derive(Debug, Deserialize)]
struct GraphqlResponse {
    data: Option<PullRequestsData>,
    errors: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct PullRequestsData {
    search: SearchResult,
}

#[derive(Debug, Deserialize)]
struct ViewerResponse {
    data: Option<ViewerData>,
    errors: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ViewerData {
    viewer: Viewer,
}

#[derive(Debug, Deserialize)]
struct Viewer {
    login: String,
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    nodes: Vec<Option<PullRequest>>,
    #[serde(rename = "pageInfo")]
    page_info: PageInfo,
}

#[derive(Debug, Deserialize)]
struct PageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct PullRequest {
    title: String,
    url: String,
    number: u64,
    #[serde(rename = "mergedAt")]
    merged_at: String,
    repository: Repository,
}

#[derive(Clone, Debug, Deserialize)]
struct Repository {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
    #[serde(rename = "isPrivate")]
    is_private: bool,
    #[serde(rename = "viewerPermission")]
    viewer_permission: Option<RepositoryPermission>,
}

#[derive(Debug)]
struct RepositoryGroup {
    repository: Repository,
    visible_prs: Vec<PullRequest>,
    has_more: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum RepositoryPermission {
    Admin,
    Maintain,
    Read,
    Triage,
    Write,
}

impl PullRequest {
    fn is_external_contribution(&self, include_private: bool) -> bool {
        (include_private || !self.repository.is_private)
            && !matches!(
                self.repository.viewer_permission,
                Some(RepositoryPermission::Admin | RepositoryPermission::Maintain)
            )
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let config = parse_args()?;
    let token = env::var("RECENT_MERGED_PR_TOKEN").map_err(|_| "Set RECENT_MERGED_PR_TOKEN")?;
    let login = resolve_login(&token)?;

    let since = (Utc::now() - Duration::days(config.lookback_days))
        .date_naive()
        .format("%Y-%m-%d");
    let search_query = format!(
        "author:{} is:pr is:merged merged:>={} -user:{} sort:updated-desc",
        login, since, login
    );

    let mut prs = fetch_merged_prs(&token, &search_query, config.max_pages)?;
    prs.retain(|pr| pr.is_external_contribution(config.include_private));
    prs.sort_by(|left, right| right.merged_at.cmp(&left.merged_at));
    let groups = group_prs_by_repository(&prs, config.repo_limit, config.max_prs_per_repo);

    let block = format_block(
        &groups,
        &login,
        Utc::now(),
        &config.marker_start,
        &config.marker_end,
    );
    if config.output_only {
        println!("{block}");
        set_action_output("markdown", &block)?;
        return Ok(());
    }

    let readme = fs::read_to_string(&config.readme)?;
    let updated = replace_block(&readme, &block, &config.marker_start, &config.marker_end)?;

    if config.check {
        print!("{updated}");
    } else {
        fs::write(&config.readme, updated)?;
    }

    Ok(())
}

fn parse_args() -> Result<Config, Box<dyn Error>> {
    let mut config = Config {
        readme: env::var("INPUT_README_PATH").unwrap_or_else(|_| "README.md".to_string()),
        marker_start: env::var("INPUT_MARKER_START").unwrap_or_else(|_| DEFAULT_START.to_string()),
        marker_end: env::var("INPUT_MARKER_END").unwrap_or_else(|_| DEFAULT_END.to_string()),
        include_private: env_bool("INPUT_INCLUDE_PRIVATE", false)?,
        repo_limit: env_usize("INPUT_REPO_LIMIT", 6)?,
        max_prs_per_repo: env_usize("INPUT_MAX_PRS_PER_REPO", 3)?,
        lookback_days: env_i64("INPUT_LOOKBACK_DAYS", 365)?,
        max_pages: env_usize("INPUT_MAX_PAGES", 10)?,
        output_only: env_bool("INPUT_OUTPUT_ONLY", false)?,
        check: false,
    };

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--readme" => config.readme = next_value(&mut args, "--readme")?,
            "--marker-start" => config.marker_start = next_value(&mut args, "--marker-start")?,
            "--marker-end" => config.marker_end = next_value(&mut args, "--marker-end")?,
            "--include-private" => config.include_private = true,
            "--repo-limit" => config.repo_limit = next_value(&mut args, "--repo-limit")?.parse()?,
            "--max-prs-per-repo" => {
                config.max_prs_per_repo = next_value(&mut args, "--max-prs-per-repo")?.parse()?
            }
            "--lookback-days" => {
                config.lookback_days = next_value(&mut args, "--lookback-days")?.parse()?
            }
            "--max-pages" => config.max_pages = next_value(&mut args, "--max-pages")?.parse()?,
            "--output-only" => config.output_only = true,
            "--check" => config.check = true,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ => return Err(format!("Unknown argument: {arg}").into()),
        }
    }

    if config.repo_limit == 0 {
        return Err("repo-limit must be greater than 0".into());
    }
    if config.max_prs_per_repo == 0 {
        return Err("max-prs-per-repo must be greater than 0".into());
    }

    Ok(config)
}

fn next_value(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<String, Box<dyn Error>> {
    args.next()
        .ok_or_else(|| format!("Missing value for {flag}").into())
}

fn env_usize(name: &str, default: usize) -> Result<usize, Box<dyn Error>> {
    match env::var(name) {
        Ok(value) if value.trim().is_empty() => Ok(default),
        Ok(value) => Ok(value.parse()?),
        Err(_) => Ok(default),
    }
}

fn env_i64(name: &str, default: i64) -> Result<i64, Box<dyn Error>> {
    match env::var(name) {
        Ok(value) => Ok(value.parse()?),
        Err(_) => Ok(default),
    }
}

fn env_bool(name: &str, default: bool) -> Result<bool, Box<dyn Error>> {
    match env::var(name) {
        Ok(value) => match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => Err(format!("{name} must be true or false").into()),
        },
        Err(_) => Ok(default),
    }
}

fn print_help() {
    println!(
        "Usage: recent-merged-pr [--readme README.md] [--repo-limit 6] \\
         [--max-prs-per-repo 3] [--lookback-days 365] [--max-pages 10] [--include-private] \\
         [--marker-start TEXT] [--marker-end TEXT] [--output-only] [--check]"
    );
}

fn resolve_login(token: &str) -> Result<String, Box<dyn Error>> {
    if let Ok(owner) = env::var("GITHUB_REPOSITORY_OWNER") {
        if !owner.trim().is_empty() {
            return Ok(owner);
        }
    }

    if let Ok(repository) = env::var("GITHUB_REPOSITORY") {
        if let Some((owner, _)) = repository.split_once('/') {
            if !owner.trim().is_empty() {
                return Ok(owner.to_string());
            }
        }
    }

    fetch_viewer_login(token)
}

fn fetch_viewer_login(token: &str) -> Result<String, Box<dyn Error>> {
    let response: ViewerResponse = ureq::post("https://api.github.com/graphql")
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .set("User-Agent", "recent-merged-pr")
        .send_json(json!({ "query": VIEWER_QUERY }))?
        .into_json()?;

    if let Some(errors) = response.errors {
        return Err(format!("GitHub API errors: {errors}").into());
    }

    Ok(response
        .data
        .ok_or("GitHub API response did not include viewer data")?
        .viewer
        .login)
}

fn fetch_merged_prs(
    token: &str,
    search_query: &str,
    max_pages: usize,
) -> Result<Vec<PullRequest>, Box<dyn Error>> {
    let agent = ureq::AgentBuilder::new().build();
    let mut prs = Vec::new();
    let mut after: Option<String> = None;

    for _ in 0..max_pages {
        let response: GraphqlResponse = agent
            .post("https://api.github.com/graphql")
            .set("Authorization", &format!("Bearer {token}"))
            .set("Content-Type", "application/json")
            .set("User-Agent", "recent-merged-pr")
            .send_json(json!({
                "query": PULL_REQUESTS_QUERY,
                "variables": {
                    "query": search_query,
                    "first": 100,
                    "after": after,
                }
            }))?
            .into_json()?;

        if let Some(errors) = response.errors {
            return Err(format!("GitHub API errors: {errors}").into());
        }

        let search = response
            .data
            .ok_or("GitHub API response did not include data")?
            .search;

        prs.extend(search.nodes.into_iter().flatten());

        if !search.page_info.has_next_page {
            break;
        }

        after = search.page_info.end_cursor;
    }

    Ok(prs)
}

fn format_block(
    groups: &[RepositoryGroup],
    author: &str,
    generated_at: DateTime<Utc>,
    marker_start: &str,
    marker_end: &str,
) -> String {
    let body = if groups.is_empty() {
        "_No merged pull requests found._".to_string()
    } else {
        groups
            .iter()
            .map(|group| format_repository_group(group, author))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "{marker_start}\n{body}\n\n_Last updated: {}_\n{marker_end}",
        generated_at.format("%Y-%m-%d %H:%M UTC")
    )
}

fn group_prs_by_repository(
    prs: &[PullRequest],
    repo_limit: usize,
    max_prs_per_repo: usize,
) -> Vec<RepositoryGroup> {
    let mut groups = Vec::new();
    let mut repository_indexes = HashMap::new();

    for pr in prs {
        if let Some(index) = repository_indexes
            .get(&pr.repository.name_with_owner)
            .copied()
        {
            let group: &mut RepositoryGroup = &mut groups[index];
            if group.visible_prs.len() < max_prs_per_repo {
                group.visible_prs.push(pr.clone());
            } else {
                group.has_more = true;
            }
            continue;
        }

        if groups.len() >= repo_limit {
            continue;
        }

        repository_indexes.insert(pr.repository.name_with_owner.clone(), groups.len());
        groups.push(RepositoryGroup {
            repository: pr.repository.clone(),
            visible_prs: vec![pr.clone()],
            has_more: false,
        });
    }

    groups
}

fn format_repository_group(group: &RepositoryGroup, author: &str) -> String {
    if group.visible_prs.len() == 1 && !group.has_more {
        return format_pull_request_line(&group.visible_prs[0]);
    }

    let mut lines = vec![format!(
        "- [{}]({})\\",
        group.repository.name_with_owner,
        repository_url(&group.repository.name_with_owner)
    )];
    let child_count = group.visible_prs.len() + usize::from(group.has_more);

    for (index, pr) in group.visible_prs.iter().enumerate() {
        let is_last = index + 1 == child_count;
        let suffix = if is_last { "" } else { "\\" };
        lines.push(format!(
            "  {} {}{}",
            tree_prefix(is_last),
            format_pull_request_child(pr),
            suffix
        ));
    }

    if group.has_more {
        lines.push(format!(
            "  {} [more...]({})",
            tree_prefix(true),
            repository_more_url(&group.repository.name_with_owner, author)
        ));
    }

    lines.join("\n")
}

fn format_pull_request_line(pr: &PullRequest) -> String {
    format!(
        "- [{}#{}]({}) · {} · merged {}",
        pr.repository.name_with_owner,
        pr.number,
        pr.url,
        clean_title(&pr.title),
        merged_date(pr)
    )
}

fn format_pull_request_child(pr: &PullRequest) -> String {
    format!(
        "[#{}]({}) · {} · merged {}",
        pr.number,
        pr.url,
        clean_title(&pr.title),
        merged_date(pr)
    )
}

fn tree_prefix(is_last: bool) -> &'static str {
    if is_last {
        "'--"
    } else {
        "|--"
    }
}

fn clean_title(title: &str) -> String {
    title.replace('\n', " ")
}

fn merged_date(pr: &PullRequest) -> &str {
    pr.merged_at.get(..10).unwrap_or(&pr.merged_at)
}

fn repository_url(name_with_owner: &str) -> String {
    format!("https://github.com/{name_with_owner}")
}

fn repository_more_url(name_with_owner: &str, author: &str) -> String {
    format!("https://github.com/{name_with_owner}/pulls?q=is%3Apr+author%3A{author}+is%3Amerged")
}

fn replace_block(
    readme: &str,
    block: &str,
    marker_start: &str,
    marker_end: &str,
) -> Result<String, Box<dyn Error>> {
    let (before, rest) = readme
        .split_once(marker_start)
        .ok_or_else(|| format!("README must contain {marker_start}"))?;
    let (_, after) = rest
        .split_once(marker_end)
        .ok_or_else(|| format!("README must contain {marker_end}"))?;
    Ok(format!("{before}{block}{after}"))
}

fn set_action_output(name: &str, value: &str) -> Result<(), Box<dyn Error>> {
    let Ok(path) = env::var("GITHUB_OUTPUT") else {
        return Ok(());
    };
    let delimiter = format!(
        "recent_merged_pr_{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let output = format!("{name}<<{delimiter}\n{value}\n{delimiter}\n");
    fs::write(path, output)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(repo: &str, number: u64, title: &str, merged_at: &str) -> PullRequest {
        PullRequest {
            title: title.to_string(),
            url: format!("https://github.com/{repo}/pull/{number}"),
            number,
            merged_at: merged_at.to_string(),
            repository: Repository {
                name_with_owner: repo.to_string(),
                is_private: false,
                viewer_permission: Some(RepositoryPermission::Read),
            },
        }
    }

    fn generated_at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-06-04T05:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn single_pr_repository_stays_on_one_line() {
        let prs = vec![pr(
            "amruthpillai/reactive-resume",
            3095,
            "fix(auth): reconcile migrated social login accounts",
            "2026-05-25T01:00:00Z",
        )];

        let groups = group_prs_by_repository(&prs, 6, 3);
        let block = format_block(
            &groups,
            "bioinformatist",
            generated_at(),
            DEFAULT_START,
            DEFAULT_END,
        );

        assert!(block.contains(
            "- [amruthpillai/reactive-resume#3095](https://github.com/amruthpillai/reactive-resume/pull/3095) · fix(auth): reconcile migrated social login accounts · merged 2026-05-25"
        ));
        assert!(!block.contains("|--"));
    }

    #[test]
    fn multiple_prs_for_one_repository_use_tree_output() {
        let prs = vec![
            pr(
                "luccahuguet/yazelix",
                605,
                "chore: add prek",
                "2026-06-04T01:00:00Z",
            ),
            pr(
                "luccahuguet/yazelix",
                603,
                "Improve tutor",
                "2026-06-02T01:00:00Z",
            ),
            pr("SYSU-SCC/sysu-thesis", 118, "docs", "2026-01-14T01:00:00Z"),
        ];

        let groups = group_prs_by_repository(&prs, 6, 3);
        let block = format_block(
            &groups,
            "bioinformatist",
            generated_at(),
            DEFAULT_START,
            DEFAULT_END,
        );

        assert!(block.contains("- [luccahuguet/yazelix](https://github.com/luccahuguet/yazelix)"));
        assert!(block.contains("  |-- [#605](https://github.com/luccahuguet/yazelix/pull/605) · chore: add prek · merged 2026-06-04\\"));
        assert!(block.contains("  '-- [#603](https://github.com/luccahuguet/yazelix/pull/603) · Improve tutor · merged 2026-06-02"));
        assert!(block.contains("- [SYSU-SCC/sysu-thesis#118](https://github.com/SYSU-SCC/sysu-thesis/pull/118) · docs · merged 2026-01-14"));
    }

    #[test]
    fn repo_limit_counts_repositories_not_pull_requests() {
        let prs = vec![
            pr("first/repo", 3, "first latest", "2026-06-04T01:00:00Z"),
            pr("first/repo", 2, "first older", "2026-06-03T01:00:00Z"),
            pr("second/repo", 1, "second", "2026-06-02T01:00:00Z"),
            pr("third/repo", 1, "third", "2026-06-01T01:00:00Z"),
        ];

        let groups = group_prs_by_repository(&prs, 2, 3);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].repository.name_with_owner, "first/repo");
        assert_eq!(groups[0].visible_prs.len(), 2);
        assert_eq!(groups[1].repository.name_with_owner, "second/repo");
    }

    #[test]
    fn hidden_repository_prs_get_more_link_without_count() {
        let prs = vec![
            pr("luccahuguet/yazelix", 605, "one", "2026-06-04T01:00:00Z"),
            pr("other/project", 10, "other", "2026-06-03T01:00:00Z"),
            pr("luccahuguet/yazelix", 603, "two", "2026-06-02T01:00:00Z"),
            pr("luccahuguet/yazelix", 600, "three", "2026-05-31T01:00:00Z"),
        ];

        let groups = group_prs_by_repository(&prs, 2, 2);
        let block = format_block(
            &groups,
            "bioinformatist",
            generated_at(),
            DEFAULT_START,
            DEFAULT_END,
        );

        assert!(block.contains("  |-- [#605](https://github.com/luccahuguet/yazelix/pull/605) · one · merged 2026-06-04\\"));
        assert!(block.contains("  |-- [#603](https://github.com/luccahuguet/yazelix/pull/603) · two · merged 2026-06-02\\"));
        assert!(block.contains("  '-- [more...](https://github.com/luccahuguet/yazelix/pulls?q=is%3Apr+author%3Abioinformatist+is%3Amerged)"));
        assert!(!block.contains("#600"));
    }
}
