//! Thèmes de couleurs du terminal (fond, texte, curseur, palette ANSI de 16 couleurs).

pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub fg: &'static str,
    pub bg: &'static str,
    pub cursor: &'static str,
    pub palette: [&'static str; 16],
}

pub const THEMES: &[Theme] = &[
    // « NOOS » : variantes sombre et claire choisies selon le mode de l'interface.
    Theme {
        id: "noos-dark",
        name: "NOOS sombre",
        fg: "#e3e5ee",
        bg: "#14151d",
        cursor: "#8fb4ff",
        palette: [
            "#1d1f2a", "#ef6f7d", "#7fd6a1", "#f0c674", "#7aa7ff", "#c79bff", "#6fd3e0", "#c9ccd8",
            "#5b6075", "#ff8b97", "#9ce8b9", "#ffd98e", "#9dbdff", "#d9b6ff", "#93e4ee", "#f4f5fa",
        ],
    },
    Theme {
        id: "noos-light",
        name: "NOOS clair",
        fg: "#23252f",
        bg: "#f6f7fb",
        cursor: "#2f6fe4",
        palette: [
            "#23252f", "#c4283c", "#1f8a4c", "#9a6a00", "#2f6fe4", "#8a3fd1", "#0f8a99", "#c9ccd8",
            "#6b7086", "#e0435a", "#2ea862", "#b88600", "#4d88f0", "#a55ee6", "#20a7b8", "#f4f5fa",
        ],
    },
    Theme {
        id: "campbell",
        name: "Campbell (Windows Terminal)",
        fg: "#cccccc",
        bg: "#0c0c0c",
        cursor: "#ffffff",
        palette: [
            "#0c0c0c", "#c50f1f", "#13a10e", "#c19c00", "#0037da", "#881798", "#3a96dd", "#cccccc",
            "#767676", "#e74856", "#16c60c", "#f9f1a5", "#3b78ff", "#b4009e", "#61d6d6", "#f2f2f2",
        ],
    },
    Theme {
        id: "one-half-dark",
        name: "One Half Dark",
        fg: "#dcdfe4",
        bg: "#282c34",
        cursor: "#a3b3cc",
        palette: [
            "#282c34", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#dcdfe4",
            "#5a6374", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#dcdfe4",
        ],
    },
    Theme {
        id: "one-half-light",
        name: "One Half Light",
        fg: "#383a42",
        bg: "#fafafa",
        cursor: "#4f525d",
        palette: [
            "#383a42", "#e45649", "#50a14f", "#c18401", "#0184bc", "#a626a4", "#0997b3", "#fafafa",
            "#4f525e", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#ffffff",
        ],
    },
    Theme {
        id: "dracula",
        name: "Dracula",
        fg: "#f8f8f2",
        bg: "#282a36",
        cursor: "#f8f8f2",
        palette: [
            "#21222c", "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd", "#f8f8f2",
            "#6272a4", "#ff6e6e", "#69ff94", "#ffffa5", "#d6acff", "#ff92df", "#a4ffff", "#ffffff",
        ],
    },
    Theme {
        id: "nord",
        name: "Nord",
        fg: "#d8dee9",
        bg: "#2e3440",
        cursor: "#d8dee9",
        palette: [
            "#3b4252", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0",
            "#4c566a", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4",
        ],
    },
    Theme {
        id: "catppuccin-mocha",
        name: "Catppuccin Mocha",
        fg: "#cdd6f4",
        bg: "#1e1e2e",
        cursor: "#f5e0dc",
        palette: [
            "#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#bac2de",
            "#585b70", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#a6adc8",
        ],
    },
    Theme {
        id: "tokyo-night",
        name: "Tokyo Night",
        fg: "#c0caf5",
        bg: "#1a1b26",
        cursor: "#c0caf5",
        palette: [
            "#15161e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6",
            "#414868", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5",
        ],
    },
    Theme {
        id: "gruvbox-dark",
        name: "Gruvbox sombre",
        fg: "#ebdbb2",
        bg: "#282828",
        cursor: "#ebdbb2",
        palette: [
            "#282828", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a", "#a89984",
            "#928374", "#fb4934", "#b8bb26", "#fabd2f", "#83a598", "#d3869b", "#8ec07c", "#ebdbb2",
        ],
    },
    Theme {
        id: "solarized-dark",
        name: "Solarized sombre",
        fg: "#839496",
        bg: "#002b36",
        cursor: "#93a1a1",
        palette: [
            "#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5",
            "#002b36", "#cb4b16", "#586e75", "#657b83", "#839496", "#6c71c4", "#93a1a1", "#fdf6e3",
        ],
    },
];

/// Choix proposés dans le menu : « noos » (automatique) puis les thèmes fixes.
pub fn choices() -> Vec<(&'static str, &'static str)> {
    let mut v = vec![("noos", "NOOS (suit la palette de Personnalisation)")];
    v.extend(THEMES.iter().filter(|t| !t.id.starts_with("noos-")).map(|t| (t.id, t.name)));
    v
}

pub fn resolve(id: &str, dark: bool) -> &'static Theme {
    let id = if id == "noos" { if dark { "noos-dark" } else { "noos-light" } } else { id };
    THEMES.iter().find(|t| t.id == id).unwrap_or(&THEMES[0])
}
