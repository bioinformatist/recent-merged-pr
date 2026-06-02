# Recent Merged PR

Show recent merged pull requests from external repositories in a README.

This action is designed for GitHub profile READMEs. It highlights merged pull
requests authored by the profile owner while excluding repositories that look
like maintained or owned work.

The motivation is simple: many generous engineers spend real time improving
other people's projects, but those contributions are easy to miss on a profile.
**This action turns that quiet community work into a small, current, readable
signal.**

## Behavior

In order, the action:

1. Resolves the profile owner from the workflow context.
2. Searches merged pull requests authored by that owner.
3. Keeps only PRs merged within `lookback-days`.
4. Excludes private repositories unless `include-private` is true.
5. Excludes repositories where the token has `ADMIN` or `MAINTAIN` permission.
6. Sorts the remaining PRs by merge time descending.
7. Keeps `limit` entries.
8. Edits the README marker block, unless `output-only` is enabled.

> [!TIP]
> Repositories where the token has `ADMIN` or `MAINTAIN` permission are treated
> as maintained or owned work. They are usually better represented as pinned
> repositories or project highlights rather than recent external contributions.

## Usage

This action requires a personal access token that represents the profile owner.
Store it as a repository secret, then pass it through the required `token`
input. The default workflow `GITHUB_TOKEN` is not enough for maintained-repo
filtering because it only represents the current repository workflow.

<details>
<summary>Creating the token secret</summary>

Create a personal access token and save it in the profile repository as an
Actions secret named `RECENT_MERGED_PR_TOKEN`.

Suggested UI path:

1. User **Settings** -> **Developer settings** -> **Personal access tokens**.
2. Create a token for this action.
3. Profile repository **Settings** -> **Secrets and variables** -> **Actions**.
4. Add a repository secret named `RECENT_MERGED_PR_TOKEN`.

For this action, a classic PAT with minimal/no extra scopes is usually the most
practical choice because it represents your user identity across public
repositories and organizations you can access. A fine-grained PAT can work when
its repository owner/access covers the repositories you need, but it may not
cover every organization-owned repository where `viewerPermission` matters.

GitHub docs:

- [Managing your personal access tokens](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens)
- [Using secrets in GitHub Actions](https://docs.github.com/en/actions/how-tos/write-workflows/choose-what-workflows-do/use-secrets)

</details>

Add markers to your README:

```md
<!-- RECENT_PRS_START -->
<!-- RECENT_PRS_END -->
```

Use the action in a workflow:

```yaml
name: Update recent merged PRs

on:
  # Every day at 00:00 UTC, because sadly community work does not pause on weekends.
  schedule:
    - cron: "0 0 * * *"
  workflow_dispatch:

permissions:
  contents: write

jobs:
  update:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: bioinformatist/recent-merged-pr@v0.1.3
        with:
          token: ${{ secrets.RECENT_MERGED_PR_TOKEN }}
          limit: 6
          lookback-days: 365
      - name: Commit changes
        run: |
          if git diff --quiet README.md; then
            exit 0
          fi
          git config user.name "github-actions[bot]"
          git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
          git add README.md
          git commit -m "chore: update recent merged pull requests"
          git push
```

The commit step is intentionally explicit. `github-actions[bot]` is the standard
bot identity used by GitHub Actions when a workflow commits generated files back
to the repository. The `git diff --quiet` guard avoids empty commits when the
generated README block did not change.

## Inputs

| Input | Default | Meaning |
| --- | --- | --- |
| `token` | required | Personal access token representing the profile owner. |
| `readme-path` | `README.md` | README path to update. |
| `limit` | `6` | Final number of PRs to display after filtering and sorting. |
| `lookback-days` | `365` | Only consider PRs merged within this many days. |
| `marker-start` | `<!-- RECENT_PRS_START -->` | Start marker for the generated block. |
| `marker-end` | `<!-- RECENT_PRS_END -->` | End marker for the generated block. |
| `include-private` | `false` | Include private repositories in the generated list. |
| `output-only` | `false` | Print and expose Markdown without editing README. |

## Output-Only Mode

Use `output-only` when you want the generated Markdown but prefer to update files
yourself.

```yaml
- id: recent-prs
  uses: bioinformatist/recent-merged-pr@v0.1.3
  with:
    token: ${{ secrets.RECENT_MERGED_PR_TOKEN }}
    output-only: true

- run: printf '%s\n' '${{ steps.recent-prs.outputs.markdown }}'
```

For local CLI use, set `RECENT_MERGED_PR_TOKEN` and run
`cargo run -- --help` for the equivalent flags.

## License

Source-available for non-commercial use. Commercial use is reserved to Yu Sun.
See [LICENSE](LICENSE).
