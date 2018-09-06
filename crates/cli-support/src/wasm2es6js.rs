extern crate base64;
extern crate tempfile;

use std::collections::HashSet;

use failure::Error;
use parity_wasm::elements::*;

pub struct Config {
    base64: bool,
    fetch_path: Option<String>,
}

pub struct Output {
    module: Module,
    base64: bool,
    fetch_path: Option<String>,
}

impl Config {
    pub fn new() -> Config {
        Config {
            base64: false,
            fetch_path: None,
        }
    }

    pub fn base64(&mut self, base64: bool) -> &mut Self {
        self.base64 = base64;
        self
    }

    pub fn fetch(&mut self, path: Option<String>) -> &mut Self {
        self.fetch_path = path;
        self
    }

    pub fn generate(&mut self, wasm: &[u8]) -> Result<Output, Error> {
        if !self.base64 && !self.fetch_path.is_some() {
            bail!("one of --base64 or --fetch is required");
        }
        let module = deserialize_buffer(wasm)?;
        Ok(Output {
            module,
            base64: self.base64,
            fetch_path: self.fetch_path.clone(),
        })
    }
}

impl Output {
    pub fn typescript(&self) -> String {
        let mut exports = format!("/* tslint:disable */\n");

        if let Some(i) = self.module.export_section() {
            let imported_functions = self
                .module
                .import_section()
                .map(|m| m.functions() as u32)
                .unwrap_or(0);
            for entry in i.entries() {
                let idx = match *entry.internal() {
                    Internal::Function(i) => i - imported_functions,
                    Internal::Memory(_) => {
                        exports.push_str(&format!(
                            "
                            export const {}: WebAssembly.Memory;
                            ",
                            entry.field()
                        ));
                        continue;
                    }
                    Internal::Table(_) => {
                        exports.push_str(&format!(
                            "
                            export const {}: WebAssembly.Table;
                            ",
                            entry.field()
                        ));
                        continue;
                    }
                    Internal::Global(_) => continue,
                };

                let functions = self
                    .module
                    .function_section()
                    .expect("failed to find function section");
                let idx = functions.entries()[idx as usize].type_ref();

                let types = self
                    .module
                    .type_section()
                    .expect("failed to find type section");
                let ty = match types.types()[idx as usize] {
                    Type::Function(ref f) => f,
                };
                let mut args = String::new();
                for (i, _) in ty.params().iter().enumerate() {
                    if i > 0 {
                        args.push_str(", ");
                    }
                    args.push((b'a' + (i as u8)) as char);
                    args.push_str(": number");
                }

                exports.push_str(&format!(
                    "
                    export function {name}({args}): {ret};
                    ",
                    name = entry.field(),
                    args = args,
                    ret = if ty.return_type().is_some() {
                        "number"
                    } else {
                        "void"
                    },
                ));
            }
        }

        if self.base64 {
            exports.push_str("export const booted: Promise<boolean>;");
        }

        return exports;
    }

    pub fn js(self) -> Result<String, Error> {
        let mut js_imports = String::new();
        let mut exports = String::new();
        let mut set_exports = String::new();
        let mut imports = String::new();

        if let Some(i) = self.module.import_section() {
            let mut set = HashSet::new();
            for entry in i.entries() {
                if !set.insert(entry.module()) {
                    continue;
                }

                let name = (b'a' + (set.len() as u8)) as char;
                js_imports.push_str(&format!(
                    "import * as import_{} from '{}';",
                    name,
                    entry.module()
                ));
                imports.push_str(&format!("'{}': import_{}, ", entry.module(), name));
            }
        }

        if let Some(i) = self.module.export_section() {
            for entry in i.entries() {
                exports.push_str("export let ");
                exports.push_str(entry.field());
                exports.push_str(";\n");
                set_exports.push_str(entry.field());
                set_exports.push_str(" = wasm.exports.");
                set_exports.push_str(entry.field());
                set_exports.push_str(";\n");
            }
        }
        let inst = format!(
            "
            WebAssembly.instantiate(bytes,{{ {imports} }})
                .then(obj => {{
                    const wasm = obj.instance;
                    {set_exports}
                }})
            ",
            imports = imports,
            set_exports = set_exports,
        );
        let (bytes, booted) = if self.base64 {
            let wasm = serialize(self.module).expect("failed to serialize");
            (
                format!(
                    "
                    let bytes;
                    const base64 = \"{base64}\";
                    if (typeof Buffer === 'undefined') {{
                        bytes = Uint8Array.from(atob(base64), c => c.charCodeAt(0));
                    }} else {{
                        bytes = Buffer.from(base64, 'base64');
                    }}
                    ",
                    base64 = base64::encode(&wasm)
                ),
                inst,
            )
        } else if let Some(ref path) = self.fetch_path {
            (
                String::new(),
                format!(
                    "
                    fetch('{path}')
                        .then(res => res.arrayBuffer())
                        .then(bytes => {inst})
                    ",
                    path = path,
                    inst = inst
                ),
            )
        } else {
            bail!("the option --base64 or --fetch is required");
        };
        Ok(format!(
            "
            {js_imports}
            {bytes}
            export const booted = {booted};
            {exports}
            ",
            bytes = bytes,
            booted = booted,
            js_imports = js_imports,
            exports = exports,
        ))
    }
}
