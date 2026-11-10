/*
 * Copyright (C) 2019 Intel Corporation. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */

//! This is the main entry point for executing WebAssembly modules.
//! Every process should have only one instance of this runtime by call
//! `Runtime::new()` or `Runtime::builder().build()` once.

use alloc::vec::Vec;
use core::ffi::c_void;

use wamr_sys::{
    mem_alloc_type_t_Alloc_With_Pool, mem_alloc_type_t_Alloc_With_System_Allocator,
    wasm_runtime_destroy, wasm_runtime_full_init, wasm_runtime_init, NativeSymbol,
    RunningMode_Mode_Interp, RunningMode_Mode_LLVM_JIT, RuntimeInitArgs,
};

use crate::{host_function::HostFunctionList, RuntimeError};

#[allow(dead_code)]
#[derive(Debug)]
pub struct Runtime {
    host_functions: HostFunctionList,
}

impl Runtime {
    /// return a `RuntimeBuilder` instance
    ///
    /// has to
    /// - select a allocation mode
    /// - select a running mode
    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder::default()
    }

    /// create a new `Runtime` instance with the default configuration which includes:
    /// - system allocator mode
    /// - the default running mode
    ///
    /// # Errors
    ///
    /// if the runtime initialization failed, it will return `RuntimeError::InitializationFailure`
    pub fn new() -> Result<Self, RuntimeError> {
        match unsafe { wasm_runtime_init() } {
            true => Ok(Runtime {
                host_functions: HostFunctionList::new("empty"),
            }),
            false => Err(RuntimeError::InitializationFailure),
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe {
            wasm_runtime_destroy();
        }
    }
}

/// The builder of `Runtime`. It is used to configure the runtime.
/// Get one via `Runtime::builder()`
pub struct RuntimeBuilder {
    args: RuntimeInitArgs,
    host_functions: HostFunctionList,
}

/// Can't build() until config allocator mode
impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new("host")
    }
}

impl RuntimeBuilder {
    /// create a named module runtime builder
    pub fn new(name: &str) -> Self {
        Self {
            args: RuntimeInitArgs::default(),
            host_functions: HostFunctionList::new(name),
        }
    }

    /// system allocator mode
    /// allocate memory from system allocator for runtime consumed memory
    pub fn use_system_allocator(mut self) -> RuntimeBuilder {
        self.args.mem_alloc_type = mem_alloc_type_t_Alloc_With_System_Allocator;
        self
    }

    /// system allocator mode
    /// allocate memory from pool, as a pre-allocated buffer, for runtime consumed memory
    pub fn use_memory_pool(mut self, mut pool: Vec<u8>) -> RuntimeBuilder {
        self.args.mem_alloc_type = mem_alloc_type_t_Alloc_With_Pool;
        self.args.mem_alloc_option.pool.heap_buf = pool.as_mut_ptr() as *mut c_void;
        self.args.mem_alloc_option.pool.heap_size = pool.len() as u32;
        self
    }

    /// use interpreter mode
    pub fn run_as_interpreter(mut self) -> RuntimeBuilder {
        self.args.running_mode = RunningMode_Mode_Interp;
        self
    }

    /// use llvm-jit mode
    pub fn run_as_llvm_jit(mut self, opt_level: u32, size_level: u32) -> RuntimeBuilder {
        self.args.running_mode = RunningMode_Mode_LLVM_JIT;
        self.args.llvm_jit_opt_level = opt_level;
        self.args.llvm_jit_size_level = size_level;
        self
    }

    /// register a host function
    pub fn register_host_function(
        mut self,
        function_name: &str,
        function_ptr: *mut c_void,
    ) -> RuntimeBuilder {
        self.host_functions
            .register_host_function(function_name, function_ptr);
        self
    }

    /// create a `Runtime` instance with the configuration
    ///
    /// # Errors
    ///
    /// if the runtime initialization failed, it will return `RuntimeError::InitializationFailure`
    pub fn build(mut self) -> Result<Runtime, RuntimeError> {
        match unsafe {
            let module_name = &(self.host_functions).get_module_name();
            self.args.native_module_name = module_name.as_ptr();

            let native_symbols = &(self.host_functions).get_native_symbols();
            self.args.n_native_symbols = native_symbols.len() as u32;
            self.args.native_symbols = native_symbols.as_ptr() as *mut NativeSymbol;

            wasm_runtime_full_init(&mut self.args)
        } {
            true => Ok(Runtime {
                host_functions: self.host_functions,
            }),
            false => Err(RuntimeError::InitializationFailure),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wamr_sys::{wasm_runtime_free, wasm_runtime_malloc};

    #[test]
    #[ignore]
    fn test_runtime_new() {
        let runtime = Runtime::new();
        assert!(runtime.is_ok());

        /* use malloc to confirm */
        let small_buf = unsafe { wasm_runtime_malloc(16) };
        assert!(!small_buf.is_null());
        unsafe { wasm_runtime_free(small_buf) };

        drop(runtime);

        /* runtime has been destroyed. malloc should be failed */
        let small_buf = unsafe { wasm_runtime_malloc(16) };
        assert!(small_buf.is_null());

        {
            let runtime = Runtime::new();
            assert!(runtime.is_ok());

            let runtime = Runtime::new();
            assert!(runtime.is_ok());

            let runtime = Runtime::new();
            assert!(runtime.is_ok());
        }

        /* runtime has been destroyed. malloc should be failed */
        let small_buf = unsafe { wasm_runtime_malloc(16) };
        assert!(small_buf.is_null());
    }

    #[test]
    fn test_runtime_builder_default() {
        // use Mode_Default
        let runtime = Runtime::builder().use_system_allocator().build();
        assert!(runtime.is_ok());

        let small_buf = unsafe { wasm_runtime_malloc(16) };
        assert!(!small_buf.is_null());
        unsafe { wasm_runtime_free(small_buf) };
    }

    #[test]
    fn test_runtime_builder_interpreter() {
        let runtime = Runtime::builder()
            .run_as_interpreter()
            .use_system_allocator()
            .build();
        assert!(runtime.is_ok());

        let small_buf = unsafe { wasm_runtime_malloc(16) };
        assert!(!small_buf.is_null());
        unsafe { wasm_runtime_free(small_buf) };
    }

    #[test]
    #[cfg(feature = "llvmjit")]
    #[ignore]
    fn test_runtime_builder_llvm_jit() {
        let runtime = Runtime::builder()
            .run_as_llvm_jit(3, 3)
            .use_system_allocator()
            .build();
        assert!(runtime.is_ok());

        let small_buf = unsafe { wasm_runtime_malloc(16) };
        assert!(!small_buf.is_null());
        unsafe { wasm_runtime_free(small_buf) };
    }
}
