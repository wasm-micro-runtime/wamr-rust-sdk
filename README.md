# wamr-rust-sdk

## WAMR Rust SDK

### Overview

WAMR Rust SDK provides Rust language bindings for WAMR. It is the wrapper
of [*wasm_export.h*](../../../core/iwasm/include/wasm_export.h) but with Rust style.
It is more convenient to use WAMR in Rust with this crate.

This crate contains API used to interact with Wasm modules. You can compile
modules, instantiate modules, call their export functions, etc.
Plus, as an embedded of Wasm, you can provide Wasm module functionality by
creating host-defined functions.

WAMR Rust SDK includes a [*wamr-sys*](../crates/wamr-sys) crate. It will search for
the WAMR runtime source in the path *../..*. And then uses `rust-bindgen` durning
the build process to make a .so.

This crate has similar concepts to the
[WebAssembly specification](https://webassembly.github.io/spec/core/).

#### Core concepts

- *Runtime*. It is the environment that hosts all the wasm modules. Each process has one runtime instance.
- *Module*. It is the compiled .wasm or .aot. It can be loaded into runtime and instantiated into instance.
- *Instance*. It is the running instance of a module. It can be used to call export functions.
- *Function*. It is the exported function.

#### WASI concepts

- *WASIArgs*. It is used to configure the WASI environment.
  - *pre-open*. All files and directories in the list will be opened before the .wasm or .aot loaded.
  - *allowed address*. All ip addresses in the *allowed address* list will be allowed to connect with a socket.
  - *allowed DNS*.

#### WAMR private concepts

- *loading linking* instead of *instantiation linking*. *instantiation linking* is
used in Wasm JS API and Wasm C API. It means that every instance has its own, maybe
variant, imports. But *loading linking* means that all instances share the same *imports*.

- *RuntimeArg*. Control runtime behavior.
  - *running mode*.
  - *allocator*.

- *NativeFunction*.

- *WasmValues*.

### Examples

#### Example: to run a wasm32-wasip1 .wasm

*wasm32-wasip1* is a most common target for Wasm. It means that the .wasm is compiled with
`cargo build --target wasm32-wasip1` or `wasi-sdk/bin/clang --target wasm32-wasip1`.

Say there is a gcd_wasm32_wasi.wasm which includes a function named *gcd*. It returns the GCD
of two parameters.

The rust code to call the function would be:

```rust
use wamr_rust_sdk::{
    runtime::Runtime, module::Module, instance::Instance, function::Function,
    value::WasmValue, RuntimeError
};
use std::path::PathBuf;

fn main() -> Result<(), RuntimeError> {
    let runtime = Runtime::new()?;

    let mut d = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    d.push("resources/test");
    d.push("gcd_wasm32_wasi.wasm");

    let module = Module::from_file(&runtime, d.as_path())?;

    let instance = Instance::new(&runtime, &module, 1024 * 64)?;

    let function = Function::find_export_func(&instance, "gcd")?;

    let params: Vec<WasmValue> = vec![WasmValue::I32(9), WasmValue::I32(27)];
    let result = function.call(&instance, &params)?;
    assert_eq!(result, WasmValue::I32(9));

    Ok(())
}
```

#### Example: more configuration for runtime

With more configuration, runtime is capable to run .wasm with variant features, like

- Wasm without WASI requirement. Usually, it means that the .wasm is compiled with `-nostdlib`
  or `--target wasm32-unknown-unknown`
- Configure runtime.
- Provides host-defined functions to meet import requirements.

Say there is an add_extra_wasm32_wasi.wasm. Its exported function, `add()`,
requires an imported function, `extra()`, during the execution. The `add()`
adds two parameters and the result of `extra()` . It is like `a + b + extra()`.

The rust code to call the *add* function is like this:

```rust
use wamr_rust_sdk::{
    runtime::Runtime, module::Module, instance::Instance, function::Function,
    value::WasmValue, RuntimeError
};
use std::path::PathBuf;
use std::ffi::c_void;

extern "C" fn extra() -> i32 {
    100
}

fn main() -> Result<(), RuntimeError> {
    let runtime = Runtime::builder()
        .use_system_allocator()
        .register_host_function("extra", extra as *mut c_void)
        .build()?;

    let mut d = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    d.push("resources/test");
    d.push("add_extra_wasm32_wasi.wasm");
    let module = Module::from_file(&runtime, d.as_path())?;

    let instance = Instance::new(&runtime, &module, 1024 * 64)?;

    let function = Function::find_export_func(&instance, "add")?;

    let params: Vec<WasmValue> = vec![WasmValue::I32(9), WasmValue::I32(27)];
    let result = function.call(&instance, &params)?;
    assert_eq!(result, WasmValue::I32(136));

    Ok(())
}
```

### Build Instructions

#### Building `wamr-sys` and `wamr-rust-sdk`

To build the `wamr-sys` and `wamr-rust-sdk` crates, follow these steps:

1. Ensure you have the Rust toolchain installed.
2. Clone the repository:

   ```sh
   git clone https://github.com/bytecodealliance/wamr-rust-sdk.git
   cd wamr-rust-sdk
   ```

3. Build the `wamr-sys` crate:

   ```sh
   cargo build -p wamr-sys
   ```

4. Build the `wamr-rust-sdk` crate:

   ```sh
   cargo build
   ```

#### Preparing a Development and Building Environment

##### For non-espidf targets

1. Prepare the Rust toolchain

2. If targeting a non-linux platform, set `WAMR_BUILD_TARGET` and `WAMR_BUILD_PLATFORM` in the `.cargo/config.toml`:

   ```toml
   [env]
   WAMR_BUILD_PLATFORM = "OS name"
   WAMR_BUILD_TARGET = "CPU architecture"
   ```

3. If targeting a platform not supplied by WAMR, refer to the [WAMR porting guide](https://github.com/bytecodealliance/wasm-micro-runtime/blob/main/doc/port_wamr.md#wamr-porting-guide) and set `WAMR_BUILD_TARGET`, `WAMR_BUILD_PLATFORM` and `WAMR_SHARED_PLATFORM_CONFIG` in the `.cargo/config.toml` properly.

##### For espidf targets

1. Get the latest information from [The Rust on ESP Book](https://docs.espressif.com/projects/rust/book/application-development/index.html).
2. Please make sure you have installed all [prerequisites](https://github.com/esp-rs/esp-idf-template?tab=readme-ov-file#prerequisites) first!
3. Generate projects from templates following the instructions in [The Rust on ESP Book](https://docs.espressif.com/projects/rust/book/getting-started/using-esp-generate.html).

   ``` sh
   $ cargo install esp-generate --locked
   $ esp-generate
   # follow prompts from the command
   ```

4. Add the following configuration to your project's `Cargo.toml`:

   ```toml
   wamr-rust-sdk = { git = "https://github.com/bytecodealliance/wamr-rust-sdk", features = ["esp-idf"] }


#### BKMs

- [Rust on ESP-IDF "Hello, World" template](https://github.com/esp-rs/esp-idf-template?tab=readme-ov-file#rust-on-esp-idf-hello-world-template) is a good example
- Ensure that `LIBCLANG_PATH` is correctly set (something like: */home/<user>/.rustup/toolchains/esp/xtensa-esp32-elf-clang/esp-<version>/esp-clang/lib*, if on a Mac). If not, there might be something wrong with your espup installation.
