//! Package manager patterns (PackageManager).

use crate::registry::Pattern;

pub const NPM_INSTALL: Pattern = Pattern {
    name: "npm-install",
    category: crate::category::Category::PackageManager,
    matchers: &["npm", "install"],
    // npm install is mostly "added N packages" + deprecation warnings. Keep the headline.
    keep: Some(&[
        "added ",
        "removed ",
        "changed ",
        "up to date",
        "audit",
        "vulnerabilities",
        "deprecated",
        "warn",
        "error",
    ]),
    drop: None,
};

pub const PNPM_INSTALL: Pattern = Pattern {
    name: "pnpm-install",
    category: crate::category::Category::PackageManager,
    matchers: &["pnpm", "install"],
    keep: Some(&[
        "Progress:",
        "Packages:",
        "+ ",
        "- ",
        "WARN",
        "ERR_PNPM",
        "deprecated",
    ]),
    drop: None,
};

pub const PIP_INSTALL: Pattern = Pattern {
    name: "pip-install",
    category: crate::category::Category::PackageManager,
    matchers: &["pip", "install"],
    // pip prints one "Collecting X" + "Downloading X" per package. Keep only the summary.
    keep: Some(&[
        "Successfully installed",
        "Successfully uninstalled",
        "ERROR:",
        "WARNING:",
        "Requirement already satisfied",
    ]),
    drop: None,
};
