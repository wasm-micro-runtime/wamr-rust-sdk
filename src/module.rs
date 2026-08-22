/*
 * Copyright (C) 2019 Intel Corporation. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */

//! .wasm compiled, in-memory representation
//! get one via `Module::from_file()` or `Module::from_vec()`

use alloc::{ffi::CString, string::String, vec::Vec};
use core::{ffi::c_char, marker::PhantomData, ptr};

use wamr_sys::{
    wasm_module_t, wasm_runtime_load, wasm_runtime_set_module_name,
    wasm_runtime_set_wasi_addr_pool, wasm_runtime_set_wasi_args,
    wasm_runtime_set_wasi_ns_lookup_pool, wasm_runtime_unload,
};

use crate::{
    helper::error_buf_to_string, helper::DEFAULT_ERROR_BUF_SIZE, runtime::Runtime,
    wasi_context::WasiCtx, RuntimeError,
};

#[allow(dead_code)]
#[derive(Debug)]
pub struct Module<'runtime> {
    name: String,
    module: wasm_module_t,
    // to keep the module content in memory
    content: Vec<u8>,
    wasi_ctx: WasiCtx,
    _phantom: PhantomData<&'runtime Runtime>,
}

impl<'runtime> Module<'runtime> {
    /// compile a module with the given wasm file path, use the file name as the module name
    ///
    /// # Error
    ///
    /// If the file does not exist or the file cannot be read, an `RuntimeError::WasmFileFSError` will be returned.
    /// If the wasm file is not a valid wasm file, an `RuntimeError::CompilationError` will be returned.
    #[cfg(feature = "std")]
    pub fn from_file(runtime: &'runtime Runtime, wasm_file: &std::path::Path) -> Result<Self, RuntimeError> {
        use std::{fs::File, io::Read};

        let name = wasm_file.file_name().unwrap().to_str().unwrap();
        let mut wasm_file = File::open(wasm_file)?;

        let mut binary: Vec<u8> = Vec::new();
        wasm_file.read_to_end(&mut binary)?;

        Self::from_vec(runtime, binary, name)
    }

    /// compile a module int the given buffer,
    ///
    /// # Error
    ///
    /// If the file does not exist or the file cannot be read, an `RuntimeError::WasmFileFSError` will be returned.
    /// If the wasm file is not a valid wasm file, an `RuntimeError::CompilationError` will be returned.
    pub fn from_vec(
        _runtime: &'runtime Runtime,
        mut content: Vec<u8>,
        name: &str,
    ) -> Result<Self, RuntimeError> {
        let mut error_buf: [c_char; DEFAULT_ERROR_BUF_SIZE] = [0; DEFAULT_ERROR_BUF_SIZE];
        let module = unsafe {
            wasm_runtime_load(
                content.as_mut_ptr(),
                content.len() as u32,
                error_buf.as_mut_ptr(),
                error_buf.len() as u32,
            )
        };

        if module.is_null() {
            match error_buf.len() {
                0 => {
                    return Err(RuntimeError::CompilationError(String::from(
                        "load module failed",
                    )))
                }
                _ => {
                    return Err(RuntimeError::CompilationError(error_buf_to_string(
                        &error_buf,
                    )))
                }
            }
        }

        unsafe {
            let name_c = CString::new(name.as_bytes()).unwrap();
            if !wasm_runtime_set_module_name(
                module,
                name_c.as_ptr() as *mut c_char,
                error_buf.as_mut_ptr(),
                error_buf.len() as u32,
            ) {
                return Err(RuntimeError::CompilationError(error_buf_to_string(
                    &error_buf,
                )));
            }
        }

        Ok(Module {
            name: String::from(name),
            module,
            content,
            wasi_ctx: WasiCtx::default(),
            _phantom: PhantomData,
        })
    }

    /// set Wasi context for a module
    ///
    /// This function should be called before `Instance::new`
    pub fn set_wasi_context(&mut self, wasi_ctx: WasiCtx) {
        self.wasi_ctx = wasi_ctx;

        let real_paths = if self.wasi_ctx.get_preopen_real_paths().is_empty() {
            ptr::null_mut()
        } else {
            self.wasi_ctx.get_preopen_real_paths().as_ptr() as *mut *const c_char
        };

        let mapped_paths = if self.wasi_ctx.get_preopen_mapped_paths().is_empty() {
            ptr::null_mut()
        } else {
            self.wasi_ctx.get_preopen_mapped_paths().as_ptr() as *mut *const c_char
        };

        let env = if self.wasi_ctx.get_env_vars().is_empty() {
            ptr::null_mut()
        } else {
            self.wasi_ctx.get_env_vars_ptrs().as_ptr() as *mut *const c_char
        };

        let args = if self.wasi_ctx.get_arguments().is_empty() {
            ptr::null_mut()
        } else {
            self.wasi_ctx.get_arguments_ptrs().as_ptr() as *mut *mut c_char
        };

        unsafe {
            wasm_runtime_set_wasi_args(
                self.get_inner_module(),
                real_paths,
                self.wasi_ctx.get_preopen_real_paths().len() as u32,
                mapped_paths,
                self.wasi_ctx.get_preopen_mapped_paths().len() as u32,
                env,
                self.wasi_ctx.get_env_vars().len() as u32,
                args,
                self.wasi_ctx.get_arguments().len() as i32,
            );

            let ns_lookup_pool = if self.wasi_ctx.get_allowed_dns().is_empty() {
                ptr::null_mut()
            } else {
                self.wasi_ctx.get_allowed_dns().as_ptr() as *mut *const c_char
            };

            wasm_runtime_set_wasi_ns_lookup_pool(
                self.get_inner_module(),
                ns_lookup_pool,
                self.wasi_ctx.get_allowed_dns().len() as u32,
            );

            let addr_pool = if self.wasi_ctx.get_allowed_address().is_empty() {
                ptr::null_mut()
            } else {
                self.wasi_ctx.get_allowed_address().as_ptr() as *mut *const c_char
            };
            wasm_runtime_set_wasi_addr_pool(
                self.get_inner_module(),
                addr_pool,
                self.wasi_ctx.get_allowed_address().len() as u32,
            );
        }
    }

    pub fn get_inner_module(&self) -> wasm_module_t {
        self.module
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }
}

impl Drop for Module<'_> {
    fn drop(&mut self) {
        unsafe {
            wasm_runtime_unload(self.module);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{helper::cstr_to_string, runtime::Runtime, wasi_context::WasiCtxBuilder};
    use std::path::{Path, PathBuf};
    use wamr_sys::wasm_runtime_get_module_name;

    #[test]
    fn test_module_not_exist() {
        let runtime = Runtime::new();
        assert!(runtime.is_ok());

        let runtime = runtime.unwrap();

        let module = Module::from_file(&runtime, Path::new("not_exist"));
        assert!(module.is_err());
    }

    #[test]
    fn test_module_from_buf() {
        let runtime = Runtime::new().unwrap();

        // (module
        //   (func (export "add") (param i32 i32) (result i32)
        //     (local.get 0)
        //     (local.get 1)
        //     (i32.add)
        //   )
        // )
        let binary = vec![
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x07, 0x01, 0x60, 0x02, 0x7f,
            0x7f, 0x01, 0x7f, 0x03, 0x02, 0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x61, 0x64, 0x64,
            0x00, 0x00, 0x0a, 0x09, 0x01, 0x07, 0x00, 0x20, 0x00, 0x20, 0x01, 0x6a, 0x0b,
        ];
        let binary = binary.into_iter().map(|c| c as u8).collect::<Vec<u8>>();

        let module = Module::from_vec(&runtime, binary, "");
        assert!(module.is_ok());
    }

    #[test]
    fn test_module_from_file() {
        let runtime = Runtime::new().unwrap();

        let mut d = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        d.push("resources/test");
        d.push("gcd_wasm32_wasi.wasm");
        let module = Module::from_file(&runtime, d.as_path());
        assert!(module.is_ok());
    }

    #[test]
    fn test_module_with_wasi_args() {
        let runtime = Runtime::new().unwrap();

        // (module
        //   (func (export "add") (param i32 i32) (result i32)
        //     (local.get 0)
        //     (local.get 1)
        //     (i32.add)
        //   )
        // )
        let binary = vec![
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x07, 0x01, 0x60, 0x02, 0x7f,
            0x7f, 0x01, 0x7f, 0x03, 0x02, 0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x61, 0x64, 0x64,
            0x00, 0x00, 0x0a, 0x09, 0x01, 0x07, 0x00, 0x20, 0x00, 0x20, 0x01, 0x6a, 0x0b,
        ];
        let binary = binary.into_iter().map(|c| c as u8).collect::<Vec<u8>>();

        let module = Module::from_vec(&runtime, binary, "add");
        assert!(module.is_ok());
        let mut module = module.unwrap();

        let wasi_ctx = WasiCtxBuilder::new()
            .set_pre_open_path(vec!["."], vec![])
            .set_env_vars(vec![])
            .set_allowed_address(vec![])
            .set_allowed_dns(vec![])
            .build();

        module.set_wasi_context(wasi_ctx);
    }

    #[test]
    fn test_module_name() -> Result<(), RuntimeError> {
        let runtime = Runtime::new()?;

        // (module
        //   (func (export "add") (param i32 i32) (result i32)
        //     (local.get 0)
        //     (local.get 1)
        //     (i32.add)
        //   )
        // )
        let binary = vec![
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x07, 0x01, 0x60, 0x02, 0x7f,
            0x7f, 0x01, 0x7f, 0x03, 0x02, 0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x61, 0x64, 0x64,
            0x00, 0x00, 0x0a, 0x09, 0x01, 0x07, 0x00, 0x20, 0x00, 0x20, 0x01, 0x6a, 0x0b,
        ];
        let binary = binary.into_iter().map(|c| c as u8).collect::<Vec<u8>>();

        let module = Module::from_vec(&runtime, binary, "add")?;

        assert_eq!(module.get_name(), "add");

        let name =
            cstr_to_string(unsafe { wasm_runtime_get_module_name(module.get_inner_module()) });
        assert_eq!(&name, module.get_name());

        Ok(())
    }
}
