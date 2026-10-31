# Contributing to Crustates

Everyone here follows the
[Code of Conduct](https://github.com/crustates/.github/blob/main/CODE_OF_CONDUCT.md).
Found a security issue? Report it privately, see
[SECURITY.md](https://github.com/crustates/.github/blob/main/SECURITY.md).

## Sign off every commit (DCO)

We use the [Developer Certificate of Origin](https://developercertificate.org).
Sign off every commit with `git commit -s`. This adds a
`Signed-off-by: Your Name <you@example.com>` line, which states that you have
the right to submit the change under the licence of the folder it touches.
A pull request with unsigned commits cannot be merged.

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org). The scope is
the product area in lower case:

```
feat(tide): add VM snapshots
fix(reef): keep the pool imported after a reboot
docs: explain the branch model
```

Pull requests are squash-merged, so the pull request title becomes the commit
on `development`. Write the title as a Conventional Commit too. release-plz
reads these messages to choose the next version and write the changelog.

## Branches

- `development` is the default branch. Branch from it and open your pull
  request against it.
- `main` holds released code only. A release is a pull request from
  `development` into `main`. release-plz then bumps the versions, writes
  `CHANGELOG.md`, tags `vX.Y.Z` and publishes the GitHub release.

## Bigger changes

Start with an issue or a Discussion. For a bigger change, write an
architecture decision record before you write the code. This covers a new
engine, a new external dependency, or a change to the API or the on-disk
format.

1. Copy `docs/adr/0000-template.md` to `docs/adr/NNNN-short-title.md`, using
   the next free number.
2. Open a pull request that contains only the ADR, and discuss it there.
3. Build the change once the ADR is merged.
