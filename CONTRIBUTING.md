# Contributing

Pull requests are welcome. Taking part means keeping to [the code of conduct](CODE_OF_CONDUCT.md).

## Before you start

- **Start from the latest commit** of the default branch, and bring your branch up to it again before you open a pull request: the working contract may have changed since, and a pull request is judged by the rules as they stand.
- The [working contract](AGENTS.md) says where code goes, what must never break, which test fits which job, and the order a feature is built in. It holds for code written by people and by tools alike.
- The [development guide](docs/development/guide.md) says how to run, test, and build Lyonoyl Mail, and how to start the mail server the tests need.
- The [architecture overview](docs/architecture/overview.md) says how it is put together.
- **A change larger than a fix starts as an issue**, with the form *An idea*, so that what it does is agreed before the work is done.

## What a pull request needs

- **`scripts/preflight.sh` is green where your machine can run it**, and the pull request says which steps it could not run; a maintainer runs it on the branch before merging. It is the whole check: formatting, lints, the Rust suite against a real mail server, the frontend suite, the layering and anonymity gates, the licenses, and the advisories. Its first step names anything this machine lacks, and how to get it.
- **A change arrives with its test.** A fix comes with the test that fails without it, and a bug found on a real message comes with a message of its shape in `tests/fixtures/eml/`: its markup kept, its words, names, and addresses replaced.
- **Every word a person reads is a catalog key.** Write the English and the other catalogs as well as you can, or leave them to the maintainers, who bring every language level and have it read before the change is merged.
- **A dependency comes with a reason**, under a license that leaves the product free to be used, changed, passed on, and sold.
- **Nothing identifies anybody** in the code, the tests, or the documents. Commit under a name of your own choosing, at an address that reaches no mailbox, such as your GitHub no-reply address—never the project's name. A pull request is merged as one commit made by the product, and the pull request stays on GitHub, under your account, as the record of your work.
- **The title follows [Conventional Commits](https://www.conventionalcommits.org/)**, such as `fix(sync): keep the cursor when a pass stops short`: it becomes the message of the commit that lands the pull request.
