# Contributing to rustyclinic

Thanks for wanting to help. rustyclinic aims to be the EMR any clinic in the
world can run for free, and it gets there through contributions like yours.

## Getting started

```sh
git clone https://github.com/fomoroller/rustyclinic
cd rustyclinic
cargo test --workspace                    # 266+ tests, all must pass
cargo clippy --workspace --all-targets    # zero warnings; unwrap/todo/dbg are denied
cargo fmt --check
```

Run the app with `cargo run -- serve all` and open <http://localhost:8080>.

Looking for something to work on? [`TODOS.md`](TODOS.md) is the live milestone
plan — items marked S (small) are good first contributions. The architecture
is documented in [`architecture.md`](architecture.md); read the invariants
table before touching write paths.

## Ground rules

- **All writes go through `rustyclinic-services` commands** — one command per
  file. Never mutate state from route handlers.
- **State changes are state-machine transitions**, never free-form status
  updates.
- **Both database backends matter.** Repository changes need SQLite and
  PostgreSQL implementations; the `backend_test!` harness runs your tests
  against both.
- **UI follows [`DESIGN.md`](DESIGN.md)** — colors, spacing, and touch-target
  rules are load-bearing (sunlight readability, shared tablets), not taste.
- CI enforces `cargo fmt`, `clippy -D warnings`, and the test suite.

## Sign your commits (DCO)

We use the [Developer Certificate of Origin](https://developercertificate.org)
instead of a CLA. You keep the copyright to your contributions; by signing off
you certify you have the right to submit the code under the project license
(AGPL-3.0-or-later).

Add a sign-off to each commit:

```sh
git commit -s
```

which appends a line like:

```
Signed-off-by: Your Name <you@example.com>
```

CI rejects pull requests containing unsigned commits. If you forgot, fix up
with `git rebase --signoff main`.

## Pull requests

- Keep PRs focused; separate refactors from behavior changes.
- New behavior needs tests. Bug fixes need a test that fails without the fix.
- Describe the clinical or operational scenario your change serves — this
  project's correctness bar is "a nurse relies on it", not "the tests pass".

## Security issues

Do not open public issues for vulnerabilities — see [`SECURITY.md`](SECURITY.md).
