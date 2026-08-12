# Public repository checklist

Complete these repository settings immediately before or after changing the
GitHub visibility to public:

- Require the CI and CodeQL checks on `main` through a branch ruleset.
- Require pull requests and block force pushes and branch deletion.
- Enable Dependabot alerts, dependency graph, secret scanning and push
  protection.
- Enable private vulnerability reporting so security reports follow
  `SECURITY.md` instead of public issues.
- Keep the default GitHub Actions token read-only and prevent workflows from
  approving pull requests.
- Review all historical Actions logs and artifacts before the visibility
  change because they become visible with the repository.
- Keep `v0.1.0` marked as a prerelease/source preview; do not present it as an
  installable build.

The source repository may be public independently of publishing installable
application artifacts. The release-artifact workflow only prepares files for
human review and never publishes a GitHub release.
