//! Language detection and Tree-sitter grammar bindings for polyglot codebases.

use std::path::Path;
use tree_sitter::Language;

/// Supported source code languages for AST-aware parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SupportedLanguage {
    /// Rust (`.rs`)
    Rust,
    /// TypeScript (`.ts`, `.mts`, `.cts`)
    TypeScript,
    /// TSX (`.tsx`)
    Tsx,
    /// JavaScript (`.js`, `.jsx`, `.mjs`, `.cjs`)
    JavaScript,
    /// Python (`.py`, `.pyi`)
    Python,
    /// Go (`.go`)
    Go,
    /// C (`.c`, `.h`)
    C,
    /// C++ (`.cpp`, `.hpp`, `.cc`, `.cxx`, `.hh`)
    Cpp,
    /// Java (`.java`)
    Java,
    /// C# (`.cs`)
    CSharp,
    /// Ruby (`.rb`, `.rake`, `.gemspec`)
    Ruby,
    /// PHP (`.php`, `.phtml`)
    Php,
    /// Swift (`.swift`)
    Swift,
    /// Elixir (`.ex`, `.exs`)
    Elixir,
    /// Lua (`.lua`)
    Lua,
    /// Bash / Shell (`.sh`, `.bash`, `.zsh`)
    Bash,
    /// Kotlin (`.kt`, `.kts`)
    Kotlin,
    /// Scala (`.scala`, `.sc`)
    Scala,
    /// Zig (`.zig`)
    Zig,
    /// Dart (`.dart`)
    Dart,
    /// SQL (`.sql`)
    Sql,
    /// YAML (`.yaml`, `.yml`)
    Yaml,
    /// Dockerfile / Containerfile (`Dockerfile`, `Containerfile`, `.dockerfile`)
    Dockerfile,
    /// Protocol Buffers (`.proto`)
    Proto,
    /// Solidity (`.sol`)
    Solidity,
    /// HTML (`.html`, `.htm`)
    Html,
    /// CSS (`.css`)
    Css,
    /// JSON (`.json`)
    Json,
    /// TOML (`.toml`)
    Toml,
    /// OCaml (`.ml`, `.mli`)
    Ocaml,
    /// Haskell (`.hs`, `.lhs`)
    Haskell,
    /// CMake (`CMakeLists.txt`, `.cmake`)
    Cmake,
    /// Makefile (`Makefile`, `makefile`, `.mk`)
    Make,
    /// Julia (`.jl`)
    Julia,
    /// GraphQL (`.graphql`, `.gql`)
    Graphql,
    /// R (`.r`, `.R`)
    R,
    /// HCL / Terraform (`.hcl`, `.tf`, `.tfvars`)
    Hcl,
    /// Nix (`.nix`)
    Nix,
    /// CUDA (`.cu`, `.cuh`)
    Cuda,
    /// Verilog / SystemVerilog (`.v`, `.sv`, `.svh`)
    Verilog,
    /// TLA+ (`.tla`)
    Tlaplus,
    /// Starlark / Bazel (`.bzl`, `BUILD`, `WORKSPACE`)
    Starlark,
    /// Azure Bicep (`.bicep`)
    Bicep,
    /// Gleam (`.gleam`)
    Gleam,
    /// PowerShell (`.ps1`, `.psm1`, `.psd1`)
    PowerShell,
    /// D (`.d`, `.di`)
    D,
    /// WGSL WebGPU Shader (`.wgsl`)
    Wgsl,
}

impl SupportedLanguage {
    /// Canonical language identifier string.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::JavaScript => "javascript",
            Self::Python => "python",
            Self::Go => "go",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Java => "java",
            Self::CSharp => "csharp",
            Self::Ruby => "ruby",
            Self::Php => "php",
            Self::Swift => "swift",
            Self::Elixir => "elixir",
            Self::Lua => "lua",
            Self::Bash => "bash",
            Self::Kotlin => "kotlin",
            Self::Scala => "scala",
            Self::Zig => "zig",
            Self::Dart => "dart",
            Self::Sql => "sql",
            Self::Yaml => "yaml",
            Self::Dockerfile => "dockerfile",
            Self::Proto => "proto",
            Self::Solidity => "solidity",
            Self::Html => "html",
            Self::Css => "css",
            Self::Json => "json",
            Self::Toml => "toml",
            Self::Ocaml => "ocaml",
            Self::Haskell => "haskell",
            Self::Cmake => "cmake",
            Self::Make => "make",
            Self::Julia => "julia",
            Self::Graphql => "graphql",
            Self::R => "r",
            Self::Hcl => "hcl",
            Self::Nix => "nix",
            Self::Cuda => "cuda",
            Self::Verilog => "verilog",
            Self::Tlaplus => "tlaplus",
            Self::Starlark => "starlark",
            Self::Bicep => "bicep",
            Self::Gleam => "gleam",
            Self::PowerShell => "powershell",
            Self::D => "d",
            Self::Wgsl => "wgsl",
        }
    }

    /// Return the native `tree_sitter::Language` grammar for this language.
    pub fn tree_sitter_language(&self) -> Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::JavaScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::C => tree_sitter_c::LANGUAGE.into(),
            Self::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Self::Java => tree_sitter_java::LANGUAGE.into(),
            Self::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
            Self::Ruby => tree_sitter_ruby::LANGUAGE.into(),
            Self::Php => tree_sitter_php::LANGUAGE_PHP.into(),
            Self::Swift => tree_sitter_swift::LANGUAGE.into(),
            Self::Elixir => tree_sitter_elixir::LANGUAGE.into(),
            Self::Lua => tree_sitter_lua::LANGUAGE.into(),
            Self::Bash => tree_sitter_bash::LANGUAGE.into(),
            Self::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
            Self::Scala => tree_sitter_scala::LANGUAGE.into(),
            Self::Zig => tree_sitter_zig::LANGUAGE.into(),
            Self::Dart => tree_sitter_dart::LANGUAGE.into(),
            Self::Sql => tree_sitter_sequel::LANGUAGE.into(),
            Self::Yaml => tree_sitter_yaml::LANGUAGE.into(),
            Self::Dockerfile => tree_sitter_containerfile::LANGUAGE.into(),
            Self::Proto => tree_sitter_proto::LANGUAGE.into(),
            Self::Solidity => tree_sitter_solidity::LANGUAGE.into(),
            Self::Html => tree_sitter_html::LANGUAGE.into(),
            Self::Css => tree_sitter_css::LANGUAGE.into(),
            Self::Json => tree_sitter_json::LANGUAGE.into(),
            Self::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
            Self::Ocaml => tree_sitter_ocaml::LANGUAGE_OCAML.into(),
            Self::Haskell => tree_sitter_haskell::LANGUAGE.into(),
            Self::Cmake => tree_sitter_cmake::LANGUAGE.into(),
            Self::Make => tree_sitter_make::LANGUAGE.into(),
            Self::Julia => tree_sitter_julia::LANGUAGE.into(),
            Self::Graphql => tree_sitter_graphql::LANGUAGE.into(),
            Self::R => tree_sitter_r::LANGUAGE.into(),
            Self::Hcl => tree_sitter_hcl::LANGUAGE.into(),
            Self::Nix => tree_sitter_nix::LANGUAGE.into(),
            Self::Cuda => tree_sitter_cuda::LANGUAGE.into(),
            Self::Verilog => tree_sitter_verilog::LANGUAGE.into(),
            Self::Tlaplus => tree_sitter_tlaplus::LANGUAGE.into(),
            Self::Starlark => tree_sitter_starlark::LANGUAGE.into(),
            Self::Bicep => tree_sitter_bicep::LANGUAGE.into(),
            Self::Gleam => tree_sitter_gleam::LANGUAGE.into(),
            Self::PowerShell => tree_sitter_powershell::LANGUAGE.into(),
            Self::D => tree_sitter_d::LANGUAGE.into(),
            Self::Wgsl => tree_sitter_wgsl_bevy::LANGUAGE.into(),
        }
    }

    /// Single line comment prefix for scope breadcrumbs.
    pub fn comment_prefix(&self) -> &'static str {
        crate::parser::code::spec::get_language_spec(*self).comment_prefix
    }
}

/// Detect programming language from file path extension or filename.
pub fn detect_language(path: &Path) -> Option<SupportedLanguage> {
    let filename = path.file_name()?.to_str()?.to_lowercase();
    if filename == "gemfile" || filename == "rakefile" {
        return Some(SupportedLanguage::Ruby);
    }
    if filename == "dockerfile" || filename == "containerfile" {
        return Some(SupportedLanguage::Dockerfile);
    }
    if filename == "cmakelists.txt" {
        return Some(SupportedLanguage::Cmake);
    }
    if filename == "makefile" || filename == "gnumakefile" {
        return Some(SupportedLanguage::Make);
    }
    if filename == "build"
        || filename == "build.bazel"
        || filename == "workspace"
        || filename == "workspace.bazel"
    {
        return Some(SupportedLanguage::Starlark);
    }

    let ext = path.extension()?.to_str()?.to_lowercase();
    match ext.as_str() {
        "rs" => Some(SupportedLanguage::Rust),
        "ts" | "mts" | "cts" => Some(SupportedLanguage::TypeScript),
        "tsx" => Some(SupportedLanguage::Tsx),
        "js" | "jsx" | "mjs" | "cjs" => Some(SupportedLanguage::JavaScript),
        "py" | "pyi" => Some(SupportedLanguage::Python),
        "go" => Some(SupportedLanguage::Go),
        "c" | "h" => Some(SupportedLanguage::C),
        "cpp" | "hpp" | "cc" | "cxx" | "hh" | "hxx" => Some(SupportedLanguage::Cpp),
        "java" => Some(SupportedLanguage::Java),
        "cs" => Some(SupportedLanguage::CSharp),
        "rb" | "rake" | "gemspec" => Some(SupportedLanguage::Ruby),
        "php" | "phtml" | "php3" | "php4" | "php5" | "phps" => Some(SupportedLanguage::Php),
        "swift" => Some(SupportedLanguage::Swift),
        "ex" | "exs" => Some(SupportedLanguage::Elixir),
        "lua" => Some(SupportedLanguage::Lua),
        "sh" | "bash" | "zsh" | "ksh" => Some(SupportedLanguage::Bash),
        "kt" | "kts" => Some(SupportedLanguage::Kotlin),
        "scala" | "sc" => Some(SupportedLanguage::Scala),
        "zig" => Some(SupportedLanguage::Zig),
        "dart" => Some(SupportedLanguage::Dart),
        "sql" => Some(SupportedLanguage::Sql),
        "yaml" | "yml" => Some(SupportedLanguage::Yaml),
        "dockerfile" => Some(SupportedLanguage::Dockerfile),
        "proto" => Some(SupportedLanguage::Proto),
        "sol" => Some(SupportedLanguage::Solidity),
        "html" | "htm" => Some(SupportedLanguage::Html),
        "css" => Some(SupportedLanguage::Css),
        "json" => Some(SupportedLanguage::Json),
        "toml" => Some(SupportedLanguage::Toml),
        "ml" | "mli" => Some(SupportedLanguage::Ocaml),
        "hs" | "lhs" => Some(SupportedLanguage::Haskell),
        "cmake" => Some(SupportedLanguage::Cmake),
        "mk" => Some(SupportedLanguage::Make),
        "jl" => Some(SupportedLanguage::Julia),
        "graphql" | "gql" => Some(SupportedLanguage::Graphql),
        "r" => Some(SupportedLanguage::R),
        "hcl" | "tf" | "tfvars" => Some(SupportedLanguage::Hcl),
        "nix" => Some(SupportedLanguage::Nix),
        "cu" | "cuh" => Some(SupportedLanguage::Cuda),
        "v" | "sv" | "svh" => Some(SupportedLanguage::Verilog),
        "tla" => Some(SupportedLanguage::Tlaplus),
        "bzl" | "star" => Some(SupportedLanguage::Starlark),
        "bicep" => Some(SupportedLanguage::Bicep),
        "gleam" => Some(SupportedLanguage::Gleam),
        "ps1" | "psm1" | "psd1" => Some(SupportedLanguage::PowerShell),
        "d" | "di" => Some(SupportedLanguage::D),
        "wgsl" => Some(SupportedLanguage::Wgsl),
        _ => None,
    }
}

/// Determine whether a given file path is an indexable source code file.
pub fn is_code_file(path: &Path) -> bool {
    detect_language(path).is_some()
}
