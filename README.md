# Recent Merged PR

Show recent merged pull requests from external repositories in a README.

This action is designed for GitHub profile READMEs. It highlights merged pull
requests authored by the profile owner while excluding repositories that look
like maintained or owned work.

## Behavior

- Finds pull requests authored by the repository owner.
- Keeps pull requests merged within `lookback-days`.
- Excludes private repositories by default.
- Excludes repositories where the token has `ADMIN` or `MAINTAIN` permission.
- Sorts by merge time descending and keeps `limit` entries.
- Replaces the README block between configured markers.

Repositories where the token has `ADMIN` or `MAINTAIN` permission are treated as
maintained or owned work. They are usually better represented as pinned
repositories or project highlights rather than recent external contributions.

## Usage

Add markers to your README:

```md
<!-- RECENT_PRS_START -->
<!-- RECENT_PRS_END -->
```

Use the action in a workflow:

```yaml
name: Update recent merged PRs

on:
  schedule:
    - cron: "17 */12 * * *"
  workflow_dispatch:

permissions:
  contents: write

jobs:
  update:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: bioinformatist/recent-merged-pr@v0.1.0
        with:
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

## Inputs

| Input | Default | Meaning |
| --- | --- | --- |
| `github-token` | `${{ github.token }}` | Token used to query GitHub GraphQL API. |
| `readme-path` | `README.md` | README path to update. |
| `limit` | `6` | Final number of PRs to display after filtering and sorting. |
| `lookback-days` | `365` | Only consider PRs merged within this many days. |
| `marker-start` | `<!-- RECENT_PRS_START -->` | Start marker for the generated block. |
| `marker-end` | `<!-- RECENT_PRS_END -->` | End marker for the generated block. |
| `include-private` | `false` | Include private repositories in the generated list. |
| `output-only` | `false` | Print and expose Markdown without editing README. |

## Output-Only Mode

```yaml
- id: recent-prs
  uses: bioinformatist/recent-merged-pr@v0.1.0
  with:
    output-only: true

- run: printf '%s\n' '${{ steps.recent-prs.outputs.markdown }}'
```

## License

MIT
