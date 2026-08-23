# Quickstart

Install rho from source and get a first answer out of it.

This page describes rho 0.1.0.

## What you need

- Rust 1.94 or later. Run `rustup update stable` to get it.
- An [OpenRouter](https://openrouter.ai) account and API key.
- Git and a C linker (`build-essential` on Debian or Ubuntu, Xcode Command Line Tools on macOS).

## Build it

Clone the repository and build the release binary.

```sh
git clone https://github.com/leduckhc/rho.git
cd rho
cargo build --release -p rho-cli
```

It produces `target/release/rho`. On this machine the binary is 10,718,816 bytes.

Add it to your path, or run it in place.

```sh
export PATH="$PWD/target/release:$PATH"
# or
alias rho="$PWD/target/release/rho"
```

Confirm the version. Every command below assumes `rho` is on your path.

```sh
rho --version
# rho 0.1.0
```

## Set your API key

OpenRouter is the easiest provider to start with.

```sh
export OPENROUTER_API_KEY="sk-or-..."
```

Without the key, rho fails immediately with:

```
set the OPENROUTER_API_KEY environment variable to your OpenRouter API key.
```

Nothing runs until you set it.

## Get a first answer

Make a file for rho to read, then ask about it.

```sh
mkdir ~/rho-firstrun && cd ~/rho-firstrun
echo "The answer to the riddle is 42." > note.txt
rho run "Read note.txt and tell me the answer to the riddle."
```

That run really happened, and it printed this:

```
rho: no model given, so using the default for openrouter: anthropic/claude-haiku-4.5. Set --model or RHO_MODEL to choose another.
I'll read the note.txt file for you.
The answer to the riddle is **42**.
```

The first line is a notice, not an error. rho writes a notice for anything it chose for you.
The answer proves two things at once: the provider replied, and rho ran the `read` tool on a
real file.

To choose the model yourself, pass `--model`.

```sh
rho run "Which is larger, 9.11 or 9.9?" --model anthropic/claude-haiku-4.5
```

Or set it once for your shell.

```sh
export RHO_MODEL="anthropic/claude-haiku-4.5"
```

## Open the terminal interface

Run `rho` with no subcommand to open the interactive interface.

```sh
rho
```

A full-screen interface opens. Type a prompt and press Enter. Type `/` to list commands.
Press Ctrl+C to cancel a running turn. Press it again while rho is idle to quit.

The top line shows your directory, your git branch, the model, and the provider.

## Use a different provider

**AWS Bedrock** needs `AWS_REGION` and the normal AWS credential chain.

```sh
export AWS_REGION="us-east-1"
# set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY, or use an IAM role
rho run "Reply with exactly: OK" --provider bedrock
```

The default Bedrock model is `global.anthropic.claude-haiku-4-5-20251001-v1:0`. The `global`
prefix routes the request across regions.

**Azure OpenAI** needs three variables.

```sh
export AZURE_OPENAI_API_KEY="..."
export AZURE_OPENAI_ENDPOINT="https://my-resource.openai.azure.com"
export AZURE_OPENAI_DEPLOYMENT="my-deployment"
rho run "Reply with exactly: OK" --provider azure
```

> **Not verified.** Azure OpenAI has unit tests and no live run behind it. Treat it as untested.

Azure has no default model. It names a deployment, not a model, and only your account holds the deployment name.

## Build a smaller binary

The default build holds the terminal interface and all three providers. A minimal build
holds OpenRouter only, with no terminal interface.

```sh
cargo build --release -p rho-cli --no-default-features --features minimal
```

The minimal binary is 7,268,272 bytes instead of 10,718,816. It has no terminal
interface, so use `rho run` only.

## What to read next

- [CLI reference](cli.md) — every flag and subcommand
- [Configuration](configuration.md) — config file, profiles, and defaults
- [Providers](providers.md) — OpenRouter, Bedrock, and Azure in detail
- [Feature status](status.md) — what works, what is partly built, and what is not built yet
