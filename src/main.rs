use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::json;
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
    limit: usize,
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
    let token = env::var("GITHUB_TOKEN")
        .or_else(|_| env::var("GH_TOKEN"))
        .map_err(|_| "Set GITHUB_TOKEN or GH_TOKEN")?;
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
    prs.truncate(config.limit);

    let block = format_block(&prs, Utc::now(), &config.marker_start, &config.marker_end);
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
        include_private: env_bool("INPUT_INCLUDE_PRIVATE", false)
            .or_else(|_| env_bool("PR_INCLUDE_PRIVATE", false))?,
        limit: env_usize("INPUT_LIMIT", 6).or_else(|_| env_usize("PR_LIMIT", 6))?,
        lookback_days: env_i64("INPUT_LOOKBACK_DAYS", 365)
            .or_else(|_| env_i64("PR_LOOKBACK_DAYS", 365))?,
        max_pages: env_usize("INPUT_MAX_PAGES", 10)
            .or_else(|_| env_usize("PR_SEARCH_MAX_PAGES", 10))?,
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
            "--limit" => config.limit = next_value(&mut args, "--limit")?.parse()?,
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
        "Usage: recent-merged-pr [--readme README.md] [--limit 6] \\
         [--lookback-days 365] [--max-pages 10] [--include-private] \\
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
    prs: &[PullRequest],
    generated_at: DateTime<Utc>,
    marker_start: &str,
    marker_end: &str,
) -> String {
    let body = if prs.is_empty() {
        "_No merged pull requests found._".to_string()
    } else {
        prs.iter()
            .map(|pr| {
                let title = pr.title.replace('\n', " ");
                let merged = pr.merged_at.get(..10).unwrap_or(&pr.merged_at);
                format!(
                    "- [{}#{}]({}) · {} · merged {}",
                    pr.repository.name_with_owner, pr.number, pr.url, title, merged
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "{marker_start}\n{body}\n\n_Last updated: {}_\n{marker_end}",
        generated_at.format("%Y-%m-%d %H:%M UTC")
    )
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
