use std::{collections::HashMap, path::PathBuf};

use anyhow::{anyhow, Result};
use itertools::Itertools;
use log::info;
use lspower::lsp::Url;
use satysfi_parser::{
    structure::{Header, Program, ProgramText, Signature, Statement},
    LineCol, Span,
};

/// オンメモリで取り扱うデータをまとめたデータ構造。
#[derive(Debug, Default)]
pub struct DocumentCache(pub HashMap<Url, DocumentData>);

impl DocumentCache {
    pub fn get(&self, url: &Url) -> Option<&DocumentData> {
        self.0.get(url)
    }

    /// dependencies の中のパッケージについてパースし、 Environment 情報の登録を行う。
    /// この操作は再帰的に行う。
    pub fn register_dependencies(&mut self, deps: &[Dependency]) {
        for dep in deps {
            if let Some(url) = &dep.url {
                // 既に登録されている url は一度読んでいるので skip
                if self.0.get(url).is_none() {
                    if let Ok(doc_data) = DocumentData::new_from_file(url) {
                        self.0.insert(url.clone(), doc_data);
                        // 上で格納したファイルの中に dependencies 情報があればクローンして取り出す
                        let dependencies = self.0.get(url).and_then(|doc| {
                            if let DocumentData::Parsed { environment, .. } = doc {
                                Some(environment.dependencies.clone())
                            } else {
                                None
                            }
                        });
                        if let Some(dependencies) = dependencies {
                            self.register_dependencies(&dependencies);
                        }
                    }
                }
            }
        }
    }
}

/// 一つのファイルに関するデータを纏めたデータ構造。
#[derive(Debug)]
pub enum DocumentData {
    /// パーサによって正常にパースできたデータ。
    Parsed {
        /// パース結果の具象構文木 + テキスト本体。
        program_text: ProgramText,
        /// このファイルで定義されている変数やコマンドに関する情報。
        environment: Environment,
    },

    /// パーサによってパースできなかったデータ。
    NotParsed {
        /// テキスト本体。
        text: String,
        /// エラー箇所。
        linecol: LineCol,
        /// エラー箇所にて期待するパターン（終端記号）列。
        expect: Vec<&'static str>,
    },
}

impl DocumentData {
    /// テキストから新たな DocumentData を作成する。
    pub fn new(text: &str, url: &Url) -> DocumentData {
        match ProgramText::parse(text) {
            Ok(program_text) => {
                let environment = Environment::from_program(&program_text, &url);
                DocumentData::Parsed {
                    program_text,
                    environment,
                }
            }
            Err((linecol, expect)) => {
                let text = text.to_owned();
                DocumentData::NotParsed {
                    text,
                    linecol,
                    expect,
                }
            }
        }
    }

    pub fn new_from_file(url: &Url) -> Result<DocumentData> {
        if let Ok(fpath) = url.to_file_path() {
            let text = std::fs::read_to_string(&fpath)?;
            Ok(DocumentData::new(&text, url))
        } else {
            Err(anyhow!("Failed to convert url to file path."))
        }
    }

    pub fn show_envs_debug(&self) {
        match self {
            DocumentData::Parsed { environment, .. } => environment.show_debug(),
            DocumentData::NotParsed { .. } => {}
        }
    }
}

/// 変数やコマンドに関する情報。
#[derive(Debug, Default)]
pub struct Environment {
    dependencies: Vec<Dependency>,
    components: Vec<Component>,
}

impl Environment {
    pub fn from_program(program_text: &ProgramText, url: &Url) -> Self {
        match &program_text.structure {
            Ok(structure) => {
                let (header, preamble) = match structure {
                    Program::Saty {
                        header, preamble, ..
                    } => (header, preamble),
                    Program::Satyh {
                        header, preamble, ..
                    } => (header, preamble),
                };
                let header = header.iter().collect_vec();
                let preamble = preamble.iter().collect_vec();
                let dependencies = Dependency::from_header(&header, program_text, url);
                let components = Component::from_preamble(&preamble, program_text, url);
                Environment {
                    dependencies,
                    components,
                }
            }
            Err(_) => Environment::default(),
        }
    }

    /// Get a reference to the environment's dependencies.
    pub fn dependencies(&self) -> &[Dependency] {
        self.dependencies.as_slice()
    }

    pub fn modules(&self) -> Vec<&Component> {
        self.components
            .iter()
            .filter(|c| matches!(c.body, ComponentBody::Module { .. }))
            .collect_vec()
    }

    pub fn variables(&self) -> Vec<&Component> {
        self.components
            .iter()
            .filter(|c| matches!(c.body, ComponentBody::Variable { .. }))
            .collect_vec()
    }

    pub fn types(&self) -> Vec<&Component> {
        self.components
            .iter()
            .filter(|c| matches!(c.body, ComponentBody::Type { .. }))
            .collect_vec()
    }

    pub fn variants(&self) -> Vec<&Component> {
        self.components
            .iter()
            .filter(|c| matches!(c.body, ComponentBody::Variant { .. }))
            .collect_vec()
    }

    pub fn inline_cmds(&self) -> Vec<&Component> {
        self.components
            .iter()
            .filter(|c| matches!(c.body, ComponentBody::InlineCmd { .. }))
            .collect_vec()
    }

    pub fn block_cmds(&self) -> Vec<&Component> {
        self.components
            .iter()
            .filter(|c| matches!(c.body, ComponentBody::BlockCmd { .. }))
            .collect_vec()
    }

    pub fn math_cmds(&self) -> Vec<&Component> {
        self.components
            .iter()
            .filter(|c| matches!(c.body, ComponentBody::MathCmd { .. }))
            .collect_vec()
    }

    pub fn variables_external(&self, module_open: &[String]) -> Vec<&Component> {
        let local = self.variables();
        let in_mods = self
            .modules()
            .iter()
            .filter(|&module| module_open.contains(&module.name))
            .map(|module| match &module.body {
                ComponentBody::Module { components } => components
                    .iter()
                    .filter(|c| matches!(c.visibility, Visibility::Public))
                    .filter(|c| matches!(c.body, ComponentBody::Variable { .. }))
                    .collect_vec(),
                _ => unreachable!(),
            })
            .concat();
        [local, in_mods].concat()
    }

    pub fn types_external(&self, module_open: &[String]) -> Vec<&Component> {
        let local = self.types();
        let in_mods = self
            .modules()
            .iter()
            .filter(|&module| module_open.contains(&module.name))
            .map(|module| match &module.body {
                ComponentBody::Module { components } => components
                    .iter()
                    .filter(|c| matches!(c.visibility, Visibility::Public))
                    .filter(|c| matches!(c.body, ComponentBody::Type { .. }))
                    .collect_vec(),
                _ => unreachable!(),
            })
            .concat();
        [local, in_mods].concat()
    }

    pub fn variants_external(&self, module_open: &[String]) -> Vec<&Component> {
        let local = self.variants();
        let in_mods = self
            .modules()
            .iter()
            .map(|module| match &module.body {
                ComponentBody::Module { components } => components
                    .iter()
                    .filter(|c| matches!(c.visibility, Visibility::Public))
                    .filter(|c| matches!(c.body, ComponentBody::Variant { .. }))
                    .collect_vec(),
                _ => unreachable!(),
            })
            .concat();
        [local, in_mods].concat()
    }

    pub fn inline_cmds_external(&self, module_open: &[String]) -> Vec<&Component> {
        let local = self.inline_cmds();

        let in_mods = self
            .modules()
            .iter()
            .filter(|&module| module_open.contains(&module.name))
            .map(|module| match &module.body {
                ComponentBody::Module { components } => components
                    .iter()
                    .filter(|c| matches!(c.visibility, Visibility::Public))
                    .filter(|c| matches!(c.body, ComponentBody::InlineCmd { .. }))
                    .collect_vec(),
                _ => unreachable!(),
            })
            .concat();

        let in_mods_direct = self
            .modules()
            .iter()
            .map(|module| match &module.body {
                ComponentBody::Module { components } => components
                    .iter()
                    .filter(|c| matches!(c.visibility, Visibility::Direct))
                    .filter(|c| matches!(c.body, ComponentBody::InlineCmd { .. }))
                    .collect_vec(),
                _ => unreachable!(),
            })
            .concat();

        [local, in_mods, in_mods_direct].concat()
    }

    pub fn block_cmds_external(&self, module_open: &[String]) -> Vec<&Component> {
        let local = self.block_cmds();

        let in_mods = self
            .modules()
            .iter()
            .filter(|&module| module_open.contains(&module.name))
            .map(|module| match &module.body {
                ComponentBody::Module { components } => components
                    .iter()
                    .filter(|c| matches!(c.visibility, Visibility::Public))
                    .filter(|c| matches!(c.body, ComponentBody::BlockCmd { .. }))
                    .collect_vec(),
                _ => unreachable!(),
            })
            .concat();

        let in_mods_direct = self
            .modules()
            .iter()
            .map(|module| match &module.body {
                ComponentBody::Module { components } => components
                    .iter()
                    .filter(|c| matches!(c.visibility, Visibility::Direct))
                    .filter(|c| matches!(c.body, ComponentBody::BlockCmd { .. }))
                    .collect_vec(),
                _ => unreachable!(),
            })
            .concat();

        [local, in_mods, in_mods_direct].concat()
    }

    pub fn math_cmds_external(&self, module_open: &[String]) -> Vec<&Component> {
        let local = self.math_cmds();

        let in_mods = self
            .modules()
            .iter()
            .filter(|&module| module_open.contains(&module.name))
            .map(|module| match &module.body {
                ComponentBody::Module { components } => components
                    .iter()
                    .filter(|c| matches!(c.visibility, Visibility::Public))
                    .filter(|c| matches!(c.body, ComponentBody::MathCmd { .. }))
                    .collect_vec(),
                _ => unreachable!(),
            })
            .concat();

        let in_mods_direct = self
            .modules()
            .iter()
            .map(|module| match &module.body {
                ComponentBody::Module { components } => components
                    .iter()
                    .filter(|c| matches!(c.visibility, Visibility::Direct))
                    .filter(|c| matches!(c.body, ComponentBody::MathCmd { .. }))
                    .collect_vec(),
                _ => unreachable!(),
            })
            .concat();

        [local, in_mods, in_mods_direct].concat()
    }

    pub fn show_debug(&self) {
        for dep in &self.dependencies {
            info!("Dependency: {:?}", dep.name);
        }
        for module in self.modules() {
            info!("Module: {:?}", module.name);
        }
        for var in self.variables() {
            info!("Varable: {:?}", var.name);
        }
        for cmd in self.inline_cmds() {
            info!("InlineCmd: {:?}", cmd.name);
        }
        for cmd in self.block_cmds() {
            info!("BlockCmd: {:?}", cmd.name);
        }
        for cmd in self.math_cmds() {
            info!("BlockCmd: {:?}", cmd.name);
        }
    }
}

#[derive(Debug, Clone)]
pub struct Dependency {
    /// パッケージ名。
    pub name: String,
    /// require か import か。
    pub kind: DependencyKind,
    /// `@require:` や `@import` が呼ばれている場所。
    pub definition: Span,
    /// 実際のファイルパス。パスを解決できなかったら None を返す。
    pub url: Option<Url>,
}

impl Dependency {
    fn from_header(headers: &[&Header], program_text: &ProgramText, url: &Url) -> Vec<Dependency> {
        let require_packages = headers.iter().map(|header| &header.name);
        let import_packages = headers.iter().map(|header| &header.name);

        let mut deps = vec![];
        let home_path = std::env::var("HOME").map(PathBuf::from).ok();
        let file_path = url.to_file_path().ok();

        // require 系のパッケージの依存関係追加
        if let Some(home_path) = home_path {
            let dist_path = home_path.join(".satysfi/dist/packages");

            let require_dependencies = require_packages.map(|pkg| {
                let pkgname = program_text.get_text(pkg);
                // TODO: consider satyg file
                let pkg_path = dist_path.join(format!("{}.satyh", pkgname));
                let url = if pkg_path.exists() {
                    Url::from_file_path(pkg_path).ok()
                } else {
                    None
                };
                Dependency {
                    name: pkgname.to_owned(),
                    kind: DependencyKind::Require,
                    definition: pkg.span,
                    url,
                }
            });

            deps.extend(require_dependencies);
        }

        if let Some(file_path) = file_path {
            // TODO: add validate
            let parent_path = file_path.parent().unwrap();

            let import_dependencies = import_packages.map(|pkg| {
                let pkgname = program_text.get_text(pkg);
                // TODO: consider satyg file
                let pkg_path = parent_path.join(format!("{}.satyh", pkgname));
                let url = if pkg_path.exists() {
                    Url::from_file_path(pkg_path).ok()
                } else {
                    None
                };
                Dependency {
                    name: pkgname.to_owned(),
                    kind: DependencyKind::Import,
                    definition: pkg.span,
                    url,
                }
            });

            deps.extend(import_dependencies);
        }

        deps
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyKind {
    Require,
    Import,
}

#[derive(Debug)]
pub struct Component {
    /// 名前。
    pub name: String,
    /// 種類。
    pub body: ComponentBody,
    /// その変数がそのファイル内で有効なスコープ。
    pub scope: Span,
    /// 定義がどこにあるか。
    pub pos_definition: Span,
    /// 可視性。パッケージ内に直接定義されていたら public。
    /// module 内のときは signature があるかどうかで変わる。
    pub visibility: Visibility,
    /// モジュール内のパブリック変数のとき、宣言がどこにあるか。
    pub pos_declaration: Option<Span>,
    /// そのコンポーネントが定義されている URL。
    pub url: Url,
}

/// モジュールについての情報。モジュール内で定義された変数を格納するのに用いる。
struct ModuleInfo<'a> {
    module_span: Span,
    sigs: &'a [Signature],
}

impl<'a> ModuleInfo<'a> {
    fn map_types<'b>(&self, program_text: &'b ProgramText) -> HashMap<&'b str, &Signature> {
        self.sigs
            .iter()
            .filter_map(|sig| match sig {
                Signature::Type { name, .. } => Some((program_text.get_text(name), sig)),
                _ => None,
            })
            .collect()
    }
    fn map_val<'b>(&self, program_text: &'b ProgramText) -> HashMap<&'b str, &Signature> {
        self.sigs
            .iter()
            .filter_map(|sig| match sig {
                Signature::Val { var, .. } => Some((program_text.get_text(var), sig)),
                _ => None,
            })
            .collect()
    }
    fn map_direct<'b>(&self, program_text: &'b ProgramText) -> HashMap<&'b str, &Signature> {
        self.sigs
            .iter()
            .filter_map(|sig| match sig {
                Signature::Direct { var, .. } => Some((program_text.get_text(var), sig)),
                _ => None,
            })
            .collect()
    }
}

impl Component {
    fn from_preamble(
        preamble: &[&Statement],
        program_text: &ProgramText,
        url: &Url,
    ) -> Vec<Component> {
        preamble
            .iter()
            .map(|stmt| Component::from_stmt(stmt, None, program_text, url))
            .concat()
    }

    fn from_struct_stmts(
        module_info: &ModuleInfo,
        struct_stmts: &[&Statement],
        program_text: &ProgramText,
        url: &Url,
    ) -> Vec<Component> {
        struct_stmts
            .iter()
            .map(|stmt| Component::from_stmt(stmt, Some(module_info), program_text, url))
            .concat()
    }

    /// Statement から Component を生成する。
    /// Component は複数出てくることもあるため、戻り値はベクトル。というのも
    /// let (x, y) = ...
    /// みたいな式では x, y という2つの Component が作成されるため。
    fn from_stmt(
        stmt: &Statement,
        module_info: Option<&ModuleInfo>,
        program_text: &ProgramText,
        url: &Url,
    ) -> Vec<Component> {
        match stmt {
            Statement::Let {
                pat,
                type_annot,
                args,
                expr,
            } => vec![],
            Statement::LetRec(_) => vec![],

            Statement::LetInline {
                var_context,
                cmd,
                args,
                expr,
            } => {
                let sig_val_map = module_info
                    .map(|info| info.map_val(program_text))
                    .unwrap_or_default();
                let sig_direct_map = module_info
                    .map(|info| info.map_direct(program_text))
                    .unwrap_or_default();
                let name = program_text.get_text(cmd).to_owned();
                let body = ComponentBody::InlineCmd;
                let start = expr.span.end;
                let end = if let Some(info) = module_info {
                    info.module_span.end
                } else {
                    program_text.cst.span.end
                };
                let scope = Span { start, end };
                let pos_definition = cmd.span;
                let (visibility, pos_declaration) = {
                    let name = program_text.get_text(cmd);
                    match (sig_direct_map.get(name), sig_val_map.get(name)) {
                        (
                            Some(Signature::Direct {
                                var,
                                signature,
                                constraint,
                            }),
                            _,
                        ) => {
                            let pos_declaration = signature.span;
                            (Visibility::Direct, Some(pos_declaration))
                        }
                        (
                            None,
                            Some(Signature::Val {
                                var,
                                signature,
                                constraint,
                            }),
                        ) => {
                            let pos_declaration = signature.span;
                            (Visibility::Public, Some(pos_declaration))
                        }
                        _ => (Visibility::Private, None),
                    }
                };
                vec![Component {
                    name,
                    body,
                    scope,
                    pos_definition,
                    visibility,
                    pos_declaration,
                    url: url.clone(),
                }]
            }

            Statement::LetBlock {
                var_context,
                cmd,
                args,
                expr,
            } => vec![],
            Statement::LetMath { cmd, args, expr } => vec![],
            Statement::LetMutable { var, expr } => vec![],
            Statement::Type(_) => vec![],

            Statement::Module {
                name: mod_name,
                signature,
                statements,
            } => {
                let name = program_text.get_text(mod_name).to_owned();
                let module_span = program_text.cst.get_parent(mod_name).unwrap().span;
                let body = {
                    let module_info = ModuleInfo {
                        module_span,
                        sigs: &signature,
                    };
                    let struct_stmt = statements.iter().collect_vec();
                    let components =
                        Component::from_struct_stmts(&module_info, &struct_stmt, program_text, url);
                    ComponentBody::Module { components }
                };
                let scope = {
                    let start = module_span.end;
                    let end = program_text.cst.span.end;
                    Span { start, end }
                };
                let pos_definition = mod_name.span;
                let visibility = Default::default();
                let pos_declaration = None;
                vec![Component {
                    name,
                    body,
                    scope,
                    pos_definition,
                    visibility,
                    pos_declaration,
                    url: url.clone(),
                }]
            }

            Statement::Open(_) => vec![],
        }
    }
}

#[derive(Debug)]
pub enum ComponentBody {
    Module {
        components: Vec<Component>,
    },
    Variable {
        /// let 式に型情報を書いている場合、その場所。
        type_declaration: Option<Span>,
    },
    Type,
    Variant {
        /// その Variant が属する型の名前。
        type_name: String,
    },
    InlineCmd,
    BlockCmd,
    MathCmd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Private,
    Direct,
}

impl Default for Visibility {
    fn default() -> Self {
        Visibility::Public
    }
}
