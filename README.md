# psbt_rust
# Rust PSBT: Secure Bitcoin Transactions

**Rust PSBT** is an implementation of the **Partially Signed Bitcoin Transaction (PSBT)** format. It adheres to the specifications outlined in **BIP-174** and **PSBT version 2** as defined in **BIP-370**.

## Key Features

- **PSBT Functionality**: Provides comprehensive PSBT-related features from the Bitcoin ecosystem.
- **Constructor Support**: Handles miniscript-based descriptors, input descriptors, and all sighash types.
- **Advanced Signing**: Covers pre-segwit, bare and nested segwit v0, taproot key and path spendings, and various forms of tweaks and commitments.

## Minimum Supported Rust Version (MSRV)

This library ensures compatibility with any combination of features on **Rust 1.56.1**. If you’re building with the MSRV, you might need to pin specific dependencies (check out `./contrib/test.sh` for the current list).

## Development Workflow

- **Just Commands**: **just** is supported for running development workflow commands. Execute `just` from your shell to explore the available sub-commands.
- **Documentation Building**: The nightly toolchain for building docs is used. You can verify your documentation changes using the following shell alias:
    
    ```bash
    alias build-docs='RUSTDOCFLAGS="--cfg docsrs" cargo +nightly rustdoc --features="$FEATURES" -- -D rustdoc::broken-intra-doc-links'
    
    ```
    

## Githooks for Error Prevention

To catch errors before running CI, githooks are provided. If you haven’t configured local githooks, you can use the ones in this repository by running the following command in the root directory:

```bash
git config --local core.hooksPath githooks/

```

Alternatively, create symlinks in your `.git/hooks` directory to any of the githooks we provide.

## Code Formatting

Code formatting using `cargo +nightly fmt`. Refer to `./rustfmt.toml` for the current configuration.
