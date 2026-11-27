/*
 * Copyright (C) 2019 Intel Corporation. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */

//! Host defined (Rust) functions, which a Wasm module can import.
//!
//! The easiest way to provide one is `RuntimeBuilder::func_wrap()`, which
//! accepts a Rust closure and deduces the Wasm signature from its types:
//!
//! ```
//! use wamr_rust_sdk::{host_function::Caller, runtime::Runtime, RuntimeError};
//!
//! fn main() -> Result<(), RuntimeError> {
//!     let runtime = Runtime::builder()
//!         .use_system_allocator()
//!         // (import "host" "add" (func (param i32 i32) (result i32)))
//!         .func_wrap("host", "add", |a: i32, b: i32| a + b)
//!         // (import "host" "log" (func (param i32 i32)))
//!         .func_wrap("host", "log", |caller: Caller<'_>, ptr: u32, len: u32| {
//!             let mut message = vec![0u8; len as usize];
//!             // an error makes the Wasm caller trap
//!             caller.read_memory(ptr as u64, &mut message)?;
//!             println!("{}", String::from_utf8_lossy(&message));
//!             Ok::<(), wamr_rust_sdk::host_function::MemoryAccessError>(())
//!         })
//!         .build()?;
//!     # drop(runtime);
//!     Ok(())
//! }
//! ```

use std::any::Any;
use std::ffi::{c_void, CString};
use std::fmt;
use std::marker::PhantomData;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

use wamr_sys::{
    wasm_exec_env_t, wasm_module_inst_t, wasm_runtime_addr_app_to_native,
    wasm_runtime_clear_exception, wasm_runtime_get_function_attachment,
    wasm_runtime_get_module_inst, wasm_runtime_register_natives_raw, wasm_runtime_set_exception,
    wasm_runtime_unregister_natives, wasm_runtime_validate_app_addr, NativeSymbol,
};

#[allow(dead_code)]
#[derive(Debug)]
struct HostFunction {
    function_name: CString,
    function_ptr: *mut c_void,
}

#[derive(Debug)]
pub struct HostFunctionList {
    pub module_name: CString,
    // keep ownership of the content of `native_symbols`
    host_functions: Vec<HostFunction>,
    pub native_symbols: Vec<NativeSymbol>,
}

impl HostFunctionList {
    pub fn new(module_name: &str) -> Self {
        HostFunctionList {
            module_name: CString::new(module_name).unwrap(),
            host_functions: Vec::new(),
            native_symbols: Vec::new(),
        }
    }

    pub fn register_host_function(&mut self, function_name: &str, function_ptr: *mut c_void) {
        self.host_functions.push(HostFunction {
            function_name: CString::new(function_name).unwrap(),
            function_ptr,
        });

        let last = self.host_functions.last().unwrap();
        self.native_symbols
            .push(pack_host_function(&(last.function_name), function_ptr));
    }

    pub fn get_native_symbols(&mut self) -> &mut Vec<NativeSymbol> {
        &mut self.native_symbols
    }

    pub fn get_module_name(&mut self) -> &CString {
        &self.module_name
    }
}

pub fn pack_host_function(function_name: &CString, function_ptr: *mut c_void) -> NativeSymbol {
    NativeSymbol {
        symbol: function_name.as_ptr(),
        func_ptr: function_ptr,
        signature: ptr::null(),
        attachment: ptr::null_mut(),
    }
}

/// The Wasm side of a host function call. A host function gets one by taking
/// a `Caller` as its first parameter.
pub struct Caller<'a> {
    exec_env: wasm_exec_env_t,
    _phantom: PhantomData<&'a mut ()>,
}

/// A range of the linear memory which is out of bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryAccessError;

impl fmt::Display for MemoryAccessError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "out of bounds memory access")
    }
}

impl std::error::Error for MemoryAccessError {}

impl Caller<'_> {
    pub fn get_inner_exec_env(&self) -> wasm_exec_env_t {
        self.exec_env
    }

    /// the instance which is calling the host function
    pub fn get_inner_instance(&self) -> wasm_module_inst_t {
        unsafe { wasm_runtime_get_module_inst(self.exec_env) }
    }

    /// return the address of `len` bytes of the linear memory of the caller,
    /// starting at `offset`
    fn memory_range(&self, offset: u64, len: usize) -> Result<*mut u8, MemoryAccessError> {
        let instance = self.get_inner_instance();

        unsafe {
            if !wasm_runtime_validate_app_addr(instance, offset, len as u64) {
                // let the host function decide whether it is fatal
                wasm_runtime_clear_exception(instance);
                return Err(MemoryAccessError);
            }

            let address = wasm_runtime_addr_app_to_native(instance, offset) as *mut u8;
            if address.is_null() && len > 0 {
                return Err(MemoryAccessError);
            }
            Ok(address)
        }
    }

    /// copy `buffer.len()` bytes out of the linear memory of the caller,
    /// starting at `offset`
    pub fn read_memory(&self, offset: u64, buffer: &mut [u8]) -> Result<(), MemoryAccessError> {
        let address = self.memory_range(offset, buffer.len())?;
        if !buffer.is_empty() {
            unsafe { ptr::copy_nonoverlapping(address, buffer.as_mut_ptr(), buffer.len()) };
        }
        Ok(())
    }

    /// copy `data` into the linear memory of the caller, starting at `offset`
    pub fn write_memory(&mut self, offset: u64, data: &[u8]) -> Result<(), MemoryAccessError> {
        let address = self.memory_range(offset, data.len())?;
        if !data.is_empty() {
            unsafe { ptr::copy_nonoverlapping(data.as_ptr(), address, data.len()) };
        }
        Ok(())
    }
}

mod private {
    pub trait Sealed {}
}

/// A Rust type which is a Wasm value type, and so can be a parameter or the
/// result of a host function: `i32`, `u32`, `i64`, `u64`, `f32` and `f64`.
pub trait WasmType: private::Sealed + Copy + 'static {
    #[doc(hidden)]
    const SIGNATURE: char;

    /// # Safety
    ///
    /// `slot` has to be a slot of the arguments of a raw native function
    #[doc(hidden)]
    unsafe fn read(slot: *const u64) -> Self;

    /// # Safety
    ///
    /// `slot` has to be a slot of the arguments of a raw native function
    #[doc(hidden)]
    unsafe fn write(self, slot: *mut u64);
}

macro_rules! impl_wasm_type {
    ($($ty:ty => $signature:literal),* $(,)?) => {
        $(
            impl private::Sealed for $ty {}

            impl WasmType for $ty {
                const SIGNATURE: char = $signature;

                // WAMR stores a value at the address of its slot, whatever its size
                unsafe fn read(slot: *const u64) -> Self {
                    (slot as *const $ty).read()
                }

                unsafe fn write(self, slot: *mut u64) {
                    (slot as *mut $ty).write(self)
                }
            }
        )*
    };
}

impl_wasm_type! {
    i32 => 'i',
    u32 => 'i',
    i64 => 'I',
    u64 => 'I',
    f32 => 'f',
    f64 => 'F',
}

/// What a host function can return: nothing, a [`WasmType`], or a `Result` of
/// one of those. An `Err` makes the Wasm caller trap with the error as message.
pub trait WasmResult: private::Sealed {
    #[doc(hidden)]
    const SIGNATURE: Option<char>;

    /// # Safety
    ///
    /// `slot` has to be the first slot of the arguments of a raw native function
    #[doc(hidden)]
    unsafe fn finish(self, slot: *mut u64) -> Result<(), String>;
}

impl private::Sealed for () {}

impl WasmResult for () {
    const SIGNATURE: Option<char> = None;

    unsafe fn finish(self, _slot: *mut u64) -> Result<(), String> {
        Ok(())
    }
}

impl<T: WasmType> WasmResult for T {
    const SIGNATURE: Option<char> = Some(T::SIGNATURE);

    unsafe fn finish(self, slot: *mut u64) -> Result<(), String> {
        self.write(slot);
        Ok(())
    }
}

impl<T, E> private::Sealed for Result<T, E> {}

impl<E: fmt::Display> WasmResult for Result<(), E> {
    const SIGNATURE: Option<char> = None;

    unsafe fn finish(self, _slot: *mut u64) -> Result<(), String> {
        self.map_err(|e| e.to_string())
    }
}

impl<T: WasmType, E: fmt::Display> WasmResult for Result<T, E> {
    const SIGNATURE: Option<char> = Some(T::SIGNATURE);

    unsafe fn finish(self, slot: *mut u64) -> Result<(), String> {
        self.map(|value| value.write(slot))
            .map_err(|e| e.to_string())
    }
}

/// A Rust closure, or function, which can be registered as a host function
/// with `RuntimeBuilder::func_wrap()`.
///
/// It is implemented for the closures taking up to 12 [`WasmType`] parameters,
/// optionally preceded by a [`Caller`], and returning a [`WasmResult`].
pub trait IntoHostFunc<Params>: Send + Sync + Sized + 'static {
    /// the signature of the function, as WAMR writes it: "(iI)f"
    #[doc(hidden)]
    fn signature() -> String;

    /// The raw native function registered in WAMR. It finds the closure in
    /// the attachment of the native symbol.
    ///
    /// # Safety
    ///
    /// Only WAMR can call it, as a raw native function whose attachment is a
    /// `Self` and whose Wasm type matches `Self::signature()`.
    #[doc(hidden)]
    unsafe extern "C" fn trampoline(exec_env: wasm_exec_env_t, args: *mut u64);
}

fn make_signature(params: &[char], result: Option<char>) -> String {
    let mut signature = String::with_capacity(params.len() + 3);
    signature.push('(');
    signature.extend(params);
    signature.push(')');
    signature.extend(result);
    signature
}

/// run the closure, then give its result, error or panic back to WAMR
unsafe fn finish_call<R: WasmResult>(
    exec_env: wasm_exec_env_t,
    args: *mut u64,
    call: impl FnOnce() -> R,
) {
    let error = match catch_unwind(AssertUnwindSafe(call)) {
        Ok(result) => match result.finish(args) {
            Ok(()) => return,
            Err(error) => error,
        },
        // unwinding into WAMR is undefined behavior
        Err(_) => String::from("host function panicked"),
    };

    // an interior NUL can't be given to WAMR, cut the message there
    let error = error.split('\0').next().unwrap_or_default();
    let error = CString::new(error).unwrap_or_default();
    wasm_runtime_set_exception(wasm_runtime_get_module_inst(exec_env), error.as_ptr());
}

macro_rules! impl_into_host_func {
    ($($param:ident),*) => {
        impl<F, R, $($param,)*> IntoHostFunc<($($param,)*)> for F
        where
            F: Fn($($param),*) -> R + Send + Sync + 'static,
            R: WasmResult,
            $($param: WasmType,)*
        {
            fn signature() -> String {
                make_signature(&[$($param::SIGNATURE),*], R::SIGNATURE)
            }

            #[allow(non_snake_case, unused_mut, unused_assignments, unused_variables)]
            unsafe extern "C" fn trampoline(exec_env: wasm_exec_env_t, args: *mut u64) {
                let func = &*(wasm_runtime_get_function_attachment(exec_env) as *const F);
                let mut slot = args as *const u64;
                $(
                    let $param = $param::read(slot);
                    slot = slot.add(1);
                )*
                finish_call(exec_env, args, || func($($param),*));
            }
        }

        impl<F, R, $($param,)*> IntoHostFunc<(Caller<'static>, $($param,)*)> for F
        where
            F: for<'a> Fn(Caller<'a>, $($param),*) -> R + Send + Sync + 'static,
            R: WasmResult,
            $($param: WasmType,)*
        {
            fn signature() -> String {
                make_signature(&[$($param::SIGNATURE),*], R::SIGNATURE)
            }

            #[allow(non_snake_case, unused_mut, unused_assignments, unused_variables)]
            unsafe extern "C" fn trampoline(exec_env: wasm_exec_env_t, args: *mut u64) {
                let func = &*(wasm_runtime_get_function_attachment(exec_env) as *const F);
                let mut slot = args as *const u64;
                $(
                    let $param = $param::read(slot);
                    slot = slot.add(1);
                )*
                let caller = Caller {
                    exec_env,
                    _phantom: PhantomData,
                };
                finish_call(exec_env, args, || func(caller, $($param),*));
            }
        }
    };
}

impl_into_host_func!();
impl_into_host_func!(A1);
impl_into_host_func!(A1, A2);
impl_into_host_func!(A1, A2, A3);
impl_into_host_func!(A1, A2, A3, A4);
impl_into_host_func!(A1, A2, A3, A4, A5);
impl_into_host_func!(A1, A2, A3, A4, A5, A6);
impl_into_host_func!(A1, A2, A3, A4, A5, A6, A7);
impl_into_host_func!(A1, A2, A3, A4, A5, A6, A7, A8);
impl_into_host_func!(A1, A2, A3, A4, A5, A6, A7, A8, A9);
impl_into_host_func!(A1, A2, A3, A4, A5, A6, A7, A8, A9, A10);
impl_into_host_func!(A1, A2, A3, A4, A5, A6, A7, A8, A9, A10, A11);
impl_into_host_func!(A1, A2, A3, A4, A5, A6, A7, A8, A9, A10, A11, A12);

/// the closures registered with `func_wrap()` for one import module name
#[derive(Debug)]
struct WrappedModule {
    module_name: CString,
    // keep ownership of what `native_symbols` points to
    function_names: Vec<CString>,
    signatures: Vec<CString>,
    closures: Vec<Box<dyn Any + Send + Sync>>,
    native_symbols: Vec<NativeSymbol>,
    registered: bool,
}

/// all the closures registered with `func_wrap()`
#[derive(Debug, Default)]
pub(crate) struct WrappedFunctionList {
    modules: Vec<WrappedModule>,
}

impl WrappedFunctionList {
    pub(crate) fn wrap<Params>(
        &mut self,
        module_name: &str,
        function_name: &str,
        func: impl IntoHostFunc<Params>,
    ) {
        fn wrap_inner<Params, F: IntoHostFunc<Params>>(
            list: &mut WrappedFunctionList,
            module_name: &str,
            function_name: &str,
            func: F,
        ) {
            let module_name = CString::new(module_name).unwrap();
            let index = match list
                .modules
                .iter()
                .position(|module| module.module_name == module_name)
            {
                Some(index) => index,
                None => {
                    list.modules.push(WrappedModule {
                        module_name,
                        function_names: Vec::new(),
                        signatures: Vec::new(),
                        closures: Vec::new(),
                        native_symbols: Vec::new(),
                        registered: false,
                    });
                    list.modules.len() - 1
                }
            };
            let module = &mut list.modules[index];

            let function_name = CString::new(function_name).unwrap();
            let signature = CString::new(F::signature()).unwrap();
            // neither a `CString` nor a `Box` moves its content when it is moved
            let closure = Box::new(func);
            module.native_symbols.push(NativeSymbol {
                symbol: function_name.as_ptr(),
                func_ptr: F::trampoline as *mut c_void,
                signature: signature.as_ptr(),
                attachment: &*closure as *const F as *mut c_void,
            });
            module.function_names.push(function_name);
            module.signatures.push(signature);
            module.closures.push(closure);
        }

        wrap_inner(self, module_name, function_name, func)
    }

    /// Register the functions in WAMR. The runtime has to be initialized, and
    /// no function can be added afterwards.
    pub(crate) fn register(&mut self) -> bool {
        for module in &mut self.modules {
            module.registered = unsafe {
                wasm_runtime_register_natives_raw(
                    module.module_name.as_ptr(),
                    module.native_symbols.as_mut_ptr(),
                    module.native_symbols.len() as u32,
                )
            };
            if !module.registered {
                return false;
            }
        }
        true
    }

    /// has to be called before the runtime is destroyed
    pub(crate) fn unregister(&mut self) {
        for module in &mut self.modules {
            if module.registered {
                unsafe {
                    wasm_runtime_unregister_natives(
                        module.module_name.as_ptr(),
                        module.native_symbols.as_mut_ptr(),
                    );
                }
                module.registered = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        function::Function, instance::Instance, module::Module, runtime::Runtime, value::WasmValue,
        RuntimeError,
    };
    use std::env;
    use std::path::PathBuf;

    extern "C" fn extra() -> i32 {
        100
    }

    #[test]
    #[ignore]
    fn test_host_function() {
        let runtime = Runtime::builder()
            .use_system_allocator()
            .register_host_function("extra", extra as *mut c_void)
            .build()
            .unwrap();

        let mut d = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        d.push("resources/test");
        d.push("add_extra_wasm32_wasi.wasm");
        let module = Module::from_file(&runtime, d.as_path());
        assert!(module.is_ok());
        let module = module.unwrap();

        let instance = Instance::new(&runtime, &module, 1024 * 64);
        assert!(instance.is_ok());
        let instance: &Instance = &instance.unwrap();

        let function = Function::find_export_func(instance, "add");
        assert!(function.is_ok());
        let function = function.unwrap();

        let params: Vec<WasmValue> = vec![WasmValue::I32(8), WasmValue::I32(8)];
        let result = function.call(instance, &params);
        assert_eq!(result.unwrap(), vec![WasmValue::I32(116)]);
    }

    // (module
    //   (import "host" "add" (func $add (param i32 i32) (result i32)))
    //   (import "host" "mix" (func $mix (param i32 i64 f32 f64) (result f64)))
    //   (import "host" "neg64" (func $neg64 (param i64) (result i64)))
    //   (import "host" "half" (func $half (param f32) (result f32)))
    //   (import "host" "count" (func $count))
    //   (import "host" "checked" (func $checked (param i32) (result i32)))
    //   (import "host" "fill" (func $fill (param i32 i32 i32)))
    //   (import "other" "seven" (func $seven (result i32)))
    //   (memory (export "memory") 1)
    //   (func (export "add") (param i32 i32) (result i32)
    //     (call $add (local.get 0) (local.get 1)))
    //   (func (export "mix") (param i32 i64 f32 f64) (result f64)
    //     (call $mix (local.get 0) (local.get 1) (local.get 2) (local.get 3)))
    //   (func (export "neg64") (param i64) (result i64)
    //     (call $neg64 (local.get 0)))
    //   (func (export "half") (param f32) (result f32)
    //     (call $half (local.get 0)))
    //   (func (export "count")
    //     (call $count))
    //   (func (export "checked") (param i32) (result i32)
    //     (call $checked (local.get 0)))
    //   (func (export "fill") (param i32 i32 i32) (result i32)
    //     (call $fill (local.get 0) (local.get 1) (local.get 2))
    //     (i32.load8_u (local.get 0)))
    //   (func (export "seven") (result i32)
    //     (call $seven))
    // )
    fn wrapped_functions_wasm() -> Vec<u8> {
        vec![
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x32, 0x09, 0x60, 0x02, 0x7f,
            0x7f, 0x01, 0x7f, 0x60, 0x04, 0x7f, 0x7e, 0x7d, 0x7c, 0x01, 0x7c, 0x60, 0x01, 0x7e,
            0x01, 0x7e, 0x60, 0x01, 0x7d, 0x01, 0x7d, 0x60, 0x00, 0x00, 0x60, 0x01, 0x7f, 0x01,
            0x7f, 0x60, 0x03, 0x7f, 0x7f, 0x7f, 0x00, 0x60, 0x00, 0x01, 0x7f, 0x60, 0x03, 0x7f,
            0x7f, 0x7f, 0x01, 0x7f, 0x02, 0x66, 0x08, 0x04, 0x68, 0x6f, 0x73, 0x74, 0x03, 0x61,
            0x64, 0x64, 0x00, 0x00, 0x04, 0x68, 0x6f, 0x73, 0x74, 0x03, 0x6d, 0x69, 0x78, 0x00,
            0x01, 0x04, 0x68, 0x6f, 0x73, 0x74, 0x05, 0x6e, 0x65, 0x67, 0x36, 0x34, 0x00, 0x02,
            0x04, 0x68, 0x6f, 0x73, 0x74, 0x04, 0x68, 0x61, 0x6c, 0x66, 0x00, 0x03, 0x04, 0x68,
            0x6f, 0x73, 0x74, 0x05, 0x63, 0x6f, 0x75, 0x6e, 0x74, 0x00, 0x04, 0x04, 0x68, 0x6f,
            0x73, 0x74, 0x07, 0x63, 0x68, 0x65, 0x63, 0x6b, 0x65, 0x64, 0x00, 0x05, 0x04, 0x68,
            0x6f, 0x73, 0x74, 0x04, 0x66, 0x69, 0x6c, 0x6c, 0x00, 0x06, 0x05, 0x6f, 0x74, 0x68,
            0x65, 0x72, 0x05, 0x73, 0x65, 0x76, 0x65, 0x6e, 0x00, 0x07, 0x03, 0x09, 0x08, 0x00,
            0x01, 0x02, 0x03, 0x04, 0x05, 0x08, 0x07, 0x05, 0x03, 0x01, 0x00, 0x01, 0x07, 0x46,
            0x09, 0x06, 0x6d, 0x65, 0x6d, 0x6f, 0x72, 0x79, 0x02, 0x00, 0x03, 0x61, 0x64, 0x64,
            0x00, 0x08, 0x03, 0x6d, 0x69, 0x78, 0x00, 0x09, 0x05, 0x6e, 0x65, 0x67, 0x36, 0x34,
            0x00, 0x0a, 0x04, 0x68, 0x61, 0x6c, 0x66, 0x00, 0x0b, 0x05, 0x63, 0x6f, 0x75, 0x6e,
            0x74, 0x00, 0x0c, 0x07, 0x63, 0x68, 0x65, 0x63, 0x6b, 0x65, 0x64, 0x00, 0x0d, 0x04,
            0x66, 0x69, 0x6c, 0x6c, 0x00, 0x0e, 0x05, 0x73, 0x65, 0x76, 0x65, 0x6e, 0x00, 0x0f,
            0x0a, 0x46, 0x08, 0x08, 0x00, 0x20, 0x00, 0x20, 0x01, 0x10, 0x00, 0x0b, 0x0c, 0x00,
            0x20, 0x00, 0x20, 0x01, 0x20, 0x02, 0x20, 0x03, 0x10, 0x01, 0x0b, 0x06, 0x00, 0x20,
            0x00, 0x10, 0x02, 0x0b, 0x06, 0x00, 0x20, 0x00, 0x10, 0x03, 0x0b, 0x04, 0x00, 0x10,
            0x04, 0x0b, 0x06, 0x00, 0x20, 0x00, 0x10, 0x05, 0x0b, 0x0f, 0x00, 0x20, 0x00, 0x20,
            0x01, 0x20, 0x02, 0x10, 0x06, 0x20, 0x00, 0x2d, 0x00, 0x00, 0x0b, 0x04, 0x00, 0x10,
            0x07, 0x0b,
        ]
    }

    fn call(instance: &Instance, name: &str, params: Vec<WasmValue>) -> Vec<WasmValue> {
        Function::find_export_func(instance, name)
            .unwrap()
            .call(instance, &params)
            .unwrap()
    }

    fn call_error(instance: &Instance, name: &str, params: Vec<WasmValue>) -> String {
        let result = Function::find_export_func(instance, name)
            .unwrap()
            .call(instance, &params);
        match result {
            Err(RuntimeError::ExecutionError(error)) => error.message,
            other => panic!("expected an execution error, got {:?}", other),
        }
    }

    #[test]
    #[ignore]
    fn test_func_wrap() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let count = Arc::new(AtomicU32::new(0));
        let count_in_host = count.clone();

        let runtime = Runtime::builder()
            .use_system_allocator()
            .func_wrap("host", "add", |a: i32, b: i32| a + b)
            .func_wrap("host", "mix", |a: u32, b: i64, c: f32, d: f64| {
                a as f64 + b as f64 + c as f64 + d
            })
            .func_wrap("host", "neg64", |a: i64| -a)
            .func_wrap("host", "half", |a: f32| a / 2.0)
            // a closure with a state
            .func_wrap("host", "count", move || {
                count_in_host.fetch_add(1, Ordering::Relaxed);
            })
            .func_wrap("host", "checked", |a: i32| match a {
                0 => Err("zero is not allowed"),
                1 => panic!("one is even worse"),
                _ => Ok(a * 2),
            })
            .func_wrap(
                "host",
                "fill",
                |mut caller: Caller<'_>, address: u32, len: u32, value: u32| {
                    caller.write_memory(address as u64, &vec![value as u8; len as usize])
                },
            )
            // a plain function, in another import module
            .func_wrap("other", "seven", seven)
            .build()
            .unwrap();

        fn seven() -> i32 {
            7
        }

        let module = Module::from_vec(&runtime, wrapped_functions_wasm(), "wrapped").unwrap();
        let instance = &Instance::new(&runtime, &module, 1024 * 64).unwrap();

        assert_eq!(
            call(
                instance,
                "add",
                vec![WasmValue::I32(40), WasmValue::I32(-2)]
            ),
            vec![WasmValue::I32(38)]
        );
        assert_eq!(
            call(
                instance,
                "mix",
                vec![
                    WasmValue::I32(-1),
                    WasmValue::I64(-0x1_0000_0000),
                    WasmValue::F32(0.5),
                    WasmValue::F64(0.25)
                ]
            ),
            // the i32 is seen as an u32 by the host function
            vec![WasmValue::F64(u32::MAX as f64 - 4294967296.0 + 0.75)]
        );
        assert_eq!(
            call(instance, "neg64", vec![WasmValue::I64(i64::MAX)]),
            vec![WasmValue::I64(-i64::MAX)]
        );
        assert_eq!(
            call(instance, "half", vec![WasmValue::F32(3.0)]),
            vec![WasmValue::F32(1.5)]
        );

        call(instance, "count", vec![]);
        call(instance, "count", vec![]);
        assert_eq!(count.load(Ordering::Relaxed), 2);

        assert_eq!(
            call(instance, "checked", vec![WasmValue::I32(21)]),
            vec![WasmValue::I32(42)]
        );
        assert_eq!(call(instance, "seven", vec![]), vec![WasmValue::I32(7)]);

        // the host function writes in the memory of its caller
        assert_eq!(
            call(
                instance,
                "fill",
                vec![WasmValue::I32(16), WasmValue::I32(4), WasmValue::I32(0xab)]
            ),
            vec![WasmValue::I32(0xab)]
        );

        // An exception stays on its instance, so use a new one for each of
        // the calls which trap.

        // the host function returns an error
        let instance = &Instance::new(&runtime, &module, 1024 * 64).unwrap();
        assert_eq!(
            call_error(instance, "checked", vec![WasmValue::I32(0)]),
            "Exception: zero is not allowed"
        );

        // the host function panics
        let instance = &Instance::new(&runtime, &module, 1024 * 64).unwrap();
        assert_eq!(
            call_error(instance, "checked", vec![WasmValue::I32(1)]),
            "Exception: host function panicked"
        );

        // the host function can't write out of the memory: one page is 65536 bytes
        let instance = &Instance::new(&runtime, &module, 1024 * 64).unwrap();
        assert_eq!(
            call_error(
                instance,
                "fill",
                vec![WasmValue::I32(65534), WasmValue::I32(4), WasmValue::I32(1)]
            ),
            "Exception: out of bounds memory access"
        );
    }

    #[test]
    #[ignore]
    fn test_func_wrap_signature_mismatch() {
        let runtime = Runtime::builder()
            .use_system_allocator()
            // the module expects (param i32 i32) (result i32)
            .func_wrap("host", "add", |a: i64, b: i64| a + b)
            .build()
            .unwrap();

        let module = Module::from_vec(&runtime, wrapped_functions_wasm(), "mismatch").unwrap();
        let instance = &Instance::new(&runtime, &module, 1024 * 64).unwrap();

        // the import isn't linked, so the host function is never called
        let error = call_error(instance, "add", vec![WasmValue::I32(40), WasmValue::I32(2)]);
        assert!(error.contains("unlinked import function"), "{}", error);
    }

    #[test]
    #[ignore]
    fn test_func_wrap_unregistered_with_runtime() {
        // keeps WAMR initialized while the other runtime comes and goes
        let _keep_alive = Runtime::new().unwrap();

        let runtime = Runtime::builder()
            .use_system_allocator()
            .func_wrap("host", "add", |a: i32, b: i32| a + b)
            .build()
            .unwrap();
        drop(runtime);

        // the closure is gone, the import can't be linked to it anymore
        let runtime = Runtime::new().unwrap();
        let module = Module::from_vec(&runtime, wrapped_functions_wasm(), "gone").unwrap();
        let instance = &Instance::new(&runtime, &module, 1024 * 64).unwrap();

        let error = call_error(instance, "add", vec![WasmValue::I32(40), WasmValue::I32(2)]);
        assert!(error.contains("unlinked import function"), "{}", error);
    }

    #[test]
    fn test_func_wrap_signature() {
        fn signature<Params, F: IntoHostFunc<Params>>(_func: &F) -> String {
            F::signature()
        }

        assert_eq!(signature(&|| {}), "()");
        assert_eq!(signature(&|_: i32, _: u32| 0i64), "(ii)I");
        assert_eq!(signature(&|_: i64, _: u64, _: f32, _: f64| 0f32), "(IIfF)f");
        assert_eq!(signature(&|_: f32| 0f64), "(f)F");
        assert_eq!(
            signature(&|_: Caller<'_>, _: i32| Ok::<u32, String>(0)),
            "(i)i"
        );
        assert_eq!(signature(&|_: Caller<'_>| Ok::<(), String>(())), "()");
    }
}
