// Bundled into web/vendor/codemirror.js (window.CM): `npm install && npm run build`.
export { basicSetup } from "codemirror";
export { EditorView, keymap } from "@codemirror/view";
export { EditorState, Compartment } from "@codemirror/state";
export { indentWithTab } from "@codemirror/commands";
export { indentUnit } from "@codemirror/language";
export { oneDark } from "@codemirror/theme-one-dark";

import { StreamLanguage } from "@codemirror/language";
import { javascript } from "@codemirror/lang-javascript";
import { python } from "@codemirror/lang-python";
import { rust } from "@codemirror/lang-rust";
import { go } from "@codemirror/lang-go";
import { html } from "@codemirror/lang-html";
import { css } from "@codemirror/lang-css";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { yaml } from "@codemirror/lang-yaml";
import { sql } from "@codemirror/lang-sql";
import { xml } from "@codemirror/lang-xml";
import { cpp } from "@codemirror/lang-cpp";
import { java } from "@codemirror/lang-java";
import { php } from "@codemirror/lang-php";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import { dockerFile } from "@codemirror/legacy-modes/mode/dockerfile";
import { nginx } from "@codemirror/legacy-modes/mode/nginx";
import { lua } from "@codemirror/legacy-modes/mode/lua";
import { ruby } from "@codemirror/legacy-modes/mode/ruby";
import { perl } from "@codemirror/legacy-modes/mode/perl";
import { properties } from "@codemirror/legacy-modes/mode/properties";
import { diff } from "@codemirror/legacy-modes/mode/diff";
import { powerShell } from "@codemirror/legacy-modes/mode/powershell";
import { swift } from "@codemirror/legacy-modes/mode/swift";
import { r } from "@codemirror/legacy-modes/mode/r";
import { protobuf } from "@codemirror/legacy-modes/mode/protobuf";
import { cmake } from "@codemirror/legacy-modes/mode/cmake";

const legacy = (mode) => () => StreamLanguage.define(mode);

const LANGUAGES = {
  JavaScript: { load: () => javascript({ jsx: true }), ext: ["js", "mjs", "cjs", "jsx"] },
  TypeScript: { load: () => javascript({ typescript: true, jsx: true }), ext: ["ts", "mts", "cts", "tsx"] },
  Python: { load: python, ext: ["py", "pyw", "pyi"], shebang: /python/ },
  Rust: { load: rust, ext: ["rs"] },
  Go: { load: go, ext: ["go"] },
  HTML: { load: html, ext: ["html", "htm", "xhtml", "vue", "svelte"] },
  CSS: { load: css, ext: ["css", "scss", "less"] },
  JSON: { load: json, ext: ["json", "jsonc", "json5", "webmanifest", "lock"], names: [".prettierrc", ".eslintrc", "composer.lock"] },
  Markdown: { load: markdown, ext: ["md", "markdown", "mdx"] },
  YAML: { load: yaml, ext: ["yml", "yaml"] },
  SQL: { load: sql, ext: ["sql"] },
  XML: { load: xml, ext: ["xml", "svg", "xsd", "xsl", "plist", "csproj"] },
  "C/C++": { load: cpp, ext: ["c", "h", "cc", "cpp", "cxx", "hpp", "hh", "ino"] },
  Java: { load: java, ext: ["java", "kt", "kts", "groovy", "gradle", "scala"] },
  PHP: { load: php, ext: ["php", "phtml"], shebang: /php/ },
  Shell: {
    load: legacy(shell),
    ext: ["sh", "bash", "zsh", "fish", "ksh"],
    names: [".bashrc", ".bash_profile", ".profile", ".zshrc", ".zprofile", ".zshenv", ".envrc", "PKGBUILD"],
    shebang: /\b(ba|z|k|fi)?sh\b/,
  },
  TOML: { load: legacy(toml), ext: ["toml"], names: ["Cargo.lock", "Pipfile"] },
  Dockerfile: { load: legacy(dockerFile), ext: ["dockerfile"], names: ["Dockerfile", "Containerfile"] },
  Nginx: { load: legacy(nginx), names: ["nginx.conf"], dirs: ["nginx"] },
  Lua: { load: legacy(lua), ext: ["lua"], shebang: /lua/ },
  Ruby: { load: legacy(ruby), ext: ["rb", "rake", "gemspec"], names: ["Gemfile", "Rakefile"], shebang: /ruby/ },
  Perl: { load: legacy(perl), ext: ["pl", "pm"], shebang: /perl/ },
  "INI / .env": { load: legacy(properties), ext: ["ini", "cfg", "conf", "env", "properties", "service", "timer", "socket", "desktop"], names: [".env", ".gitconfig", ".editorconfig", ".npmrc"] },
  Diff: { load: legacy(diff), ext: ["diff", "patch"] },
  PowerShell: { load: legacy(powerShell), ext: ["ps1", "psm1"] },
  Swift: { load: legacy(swift), ext: ["swift"] },
  R: { load: legacy(r), ext: ["r"] },
  Protobuf: { load: legacy(protobuf), ext: ["proto"] },
  CMake: { load: legacy(cmake), ext: ["cmake"], names: ["CMakeLists.txt"] },
};

/** Language for a file path (and optionally its first line, for shebangs).
 *  Returns { name, extension } or null for plain text. */
export function languageFor(path, firstLine = "") {
  const name = path.split("/").pop();
  const lower = name.toLowerCase();
  const dir = path.split("/").slice(-2, -1)[0] || "";
  const ext = lower.includes(".") ? lower.split(".").pop() : "";
  const pick = (test) => {
    for (const [languageName, language] of Object.entries(LANGUAGES)) {
      if (test(language)) return { name: languageName, extension: language.load() };
    }
    return null;
  };
  return (
    pick((language) => language.names?.includes(name)) ||
    (lower.startsWith(".env") ? pick((language) => language.names?.includes(".env")) : null) ||
    (lower.startsWith("dockerfile") ? pick((language) => language.names?.includes("Dockerfile")) : null) ||
    (ext ? pick((language) => language.ext?.includes(ext)) : null) ||
    (dir ? pick((language) => language.dirs?.includes(dir)) : null) ||
    (firstLine.startsWith("#!") ? pick((language) => language.shebang?.test(firstLine)) : null)
  );
}

export const languageNames = Object.keys(LANGUAGES);
