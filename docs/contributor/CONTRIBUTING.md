# Contributing

Numen is an **Open Source Project**

## What?

Individuals making significant and valuable contributions are given commit-access to the project. Contributions are done
via pull-requests and need to be approved by the maintainers.

> **Note:** Contributors who are part of the organization do not need to fork the repository. They can create a branch
> directly in the repository to send a pull request.

## How?

In order to build this project you need to install some dependencies, follow the instructions in [this guide](../how-to-build.md).

## Rules

There are a few basic ground-rules for contributors (including the maintainer(s) of the project):

1. **No `--force` pushes** or modifying the master & dev branch history in any way. If you need to rebase, ensure you do it in
   your own repo. No rewriting of the history after the code has been shared (e.g. through a Pull-Request).
2. **Non-master branches**, prefixed with a short name moniker (e.g. `gav-my-feature`) must be used for ongoing work.
3. **All modifications** must be made in a **pull-request** to solicit feedback from other contributors.
4. A pull-request **must not be merged until CI** has finished successfully.
5. Contributors should adhere to the [house coding style](./STYLE_GUIDE.md).

## Merge Process

### In General

* A Pull Request (PR) needs to be reviewed and approved by project maintainers.
* No PR should be merged until all reviews' comments are addressed.

### Labels

The set of labels and their description can be found [here](./github-labels.md).

### Process

1. Please use our [Pull Request Template](./PULL_REQUEST_TEMPLATE.md) and make sure all relevant information is
   reflected in your PR.
2. Please tag each PR with minimum one `A-*` label.
3. If you’re still working on your PR, please submit as “Draft”. Once a PR is ready for review change the status to
   “Open”, so that the maintainers get to review your PR. Generally PRs should sit for 48 hours in order to garner
   feedback. It may be merged before if all relevant parties had a look at it.
4. PRs will be able to be merged once all reviewers' comments are addressed and CI is successful.

**Noting breaking changes:** When breaking APIs, the PR description should mention what was changed alongside some
examples on how to change the code to make it work/compile. It should also mention potential storage migrations and if
they require some special setup aside from adding it to the list of migrations in the runtime.

## Reviewing pull requests

When reviewing a pull request, the end-goal is to suggest useful changes to the author. Reviews should finish with
approval unless there are issues that would result in:
1. Buggy behavior.
2. Undue maintenance burden.
3. Breaking with house coding style.
4. Pessimization (i.e. reduction of speed as measured in the projects benchmarks).
5. Feature reduction (i.e. it removes some aspect of functionality that a significant minority of users rely on).
6. Uselessness (i.e. it does not strictly add a feature or fix a known issue).

The reviewers are also responsible to check:

* if the PR description is well written to facilitate integration, in case it contains breaking changes.
* the PR has an impact on docs.

**Reviews may not be used as an effective veto for a PR because**:
1. There exists a somewhat cleaner/better/faster way of accomplishing the same feature/fix.
2. It does not fit well with some other contributors' longer-term vision for the project.

****

### Issues

If what you are looking for is an answer rather than proposing a new feature or fix, search
[https://substrate.stackexchange.com](https://substrate.stackexchange.com/) to see if an post already exists, and ask if
not. Please do not file support issues here.

Before opening a new issue search to see if a similar one already exists and leave a comment that you also experienced
this issue or add your specifics that are related to an existing issue.

