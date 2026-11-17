//! Icons from Primer Octicons (MIT, © GitHub Inc.): GitHub mark, note, and
//! the command-palette group icons.
//!
//! Instead of pulling in an SVG library (resvg ≈ 1–2 MB), the SVG path data
//! is rasterised here by a ~150-line scanline filler at the exact pixel size
//! needed (crisp on Retina), cached as white alpha-mask textures and tinted
//! at draw time.

use eframe::egui::{self, Color32, ColorImage, Rect, TextureHandle, TextureOptions, load::SizedTexture, pos2, vec2};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Icon {
    Github,
    Note,
    GitBranch,
    Tag,
    GitCommit,
    Repo,
    RepoPush,
    Stack,
    FileDirectory,
    Workflow,
    Alert,
    Search,
    FileSubmodule,
    History,
    Bold,
    Italic,
    Code,
    Link,
    Heading,
    ListUnordered,
    Tasklist,
    Quote,
    KebabHorizontal,
    DiffAdded,
    DiffRemoved,
    DiffRenamed,
}

// octicons 19.42.0 (path data verbatim).
const GITHUB_16: &str = "M6.766 11.328c-2.063-.25-3.516-1.734-3.516-3.656 0-.781.281-1.625.75-2.188-.203-.515-.172-1.609.063-2.062.625-.078 1.468.25 1.968.703.594-.187 1.219-.281 1.985-.281.765 0 1.39.094 1.953.265.484-.437 1.344-.765 1.969-.687.218.422.25 1.515.046 2.047.5.593.766 1.39.766 2.203 0 1.922-1.453 3.375-3.547 3.64.531.344.89 1.094.89 1.954v1.625c0 .468.391.734.86.547C13.781 14.359 16 11.53 16 8.03 16 3.61 12.406 0 7.984 0 3.563 0 0 3.61 0 8.031a7.88 7.88 0 0 0 5.172 7.422c.422.156.828-.125.828-.547v-1.25c-.219.094-.5.156-.75.156-1.031 0-1.64-.562-2.078-1.609-.172-.422-.36-.672-.719-.719-.187-.015-.25-.093-.25-.187 0-.188.313-.328.625-.328.453 0 .844.281 1.25.86.313.452.64.655 1.031.655s.641-.14 1-.5c.266-.265.47-.5.657-.656";
const GITHUB_24: &str = "M10.226 17.284c-2.965-.36-5.054-2.493-5.054-5.256 0-1.123.404-2.336 1.078-3.144-.292-.741-.247-2.314.09-2.965.898-.112 2.111.36 2.83 1.01.853-.269 1.752-.404 2.853-.404 1.1 0 1.999.135 2.807.382.696-.629 1.932-1.1 2.83-.988.315.606.36 2.179.067 2.942.72.854 1.101 2 1.101 3.167 0 2.763-2.089 4.852-5.098 5.234.763.494 1.28 1.572 1.28 2.807v2.336c0 .674.561 1.056 1.235.786 4.066-1.55 7.255-5.615 7.255-10.646C23.5 6.188 18.334 1 11.978 1 5.62 1 .5 6.188.5 12.545c0 4.986 3.167 9.12 7.435 10.669.606.225 1.19-.18 1.19-.786V20.63a2.9 2.9 0 0 1-1.078.224c-1.483 0-2.359-.808-2.987-2.313-.247-.607-.517-.966-1.034-1.033-.27-.023-.359-.135-.359-.27 0-.27.45-.471.898-.471.652 0 1.213.404 1.797 1.235.45.651.921.943 1.483.943.561 0 .92-.202 1.437-.719.382-.381.674-.718.944-.943";
const NOTE_16: &str = "M0 3.75C0 2.784.784 2 1.75 2h12.5c.966 0 1.75.784 1.75 1.75v8.5A1.75 1.75 0 0 1 14.25 14H1.75A1.75 1.75 0 0 1 0 12.25Zm1.75-.25a.25.25 0 0 0-.25.25v8.5c0 .138.112.25.25.25h12.5a.25.25 0 0 0 .25-.25v-8.5a.25.25 0 0 0-.25-.25ZM3.5 6.25a.75.75 0 0 1 .75-.75h7a.75.75 0 0 1 0 1.5h-7a.75.75 0 0 1-.75-.75Zm.75 2.25h4a.75.75 0 0 1 0 1.5h-4a.75.75 0 0 1 0-1.5Z";
const NOTE_24: &str = "M0 4.75C0 3.784.784 3 1.75 3h20.5c.966 0 1.75.784 1.75 1.75v14.5A1.75 1.75 0 0 1 22.25 21H1.75A1.75 1.75 0 0 1 0 19.25Zm1.75-.25a.25.25 0 0 0-.25.25v14.5c0 .138.112.25.25.25h20.5a.25.25 0 0 0 .25-.25V4.75a.25.25 0 0 0-.25-.25Z M5 8.75A.75.75 0 0 1 5.75 8h11.5a.75.75 0 0 1 0 1.5H5.75A.75.75 0 0 1 5 8.75Zm0 4a.75.75 0 0 1 .75-.75h5.5a.75.75 0 0 1 0 1.5h-5.5a.75.75 0 0 1-.75-.75Z";

const GIT_BRANCH_16: &str = "M9.5 3.25a2.25 2.25 0 1 1 3 2.122V6A2.5 2.5 0 0 1 10 8.5H6a1 1 0 0 0-1 1v1.128a2.251 2.251 0 1 1-1.5 0V5.372a2.25 2.25 0 1 1 1.5 0v1.836A2.493 2.493 0 0 1 6 7h4a1 1 0 0 0 1-1v-.628A2.25 2.25 0 0 1 9.5 3.25Zm-6 0a.75.75 0 1 0 1.5 0 .75.75 0 0 0-1.5 0Zm8.25-.75a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5ZM4.25 12a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5Z";
const TAG_16: &str = "M1 7.775V2.75C1 1.784 1.784 1 2.75 1h5.025c.464 0 .91.184 1.238.513l6.25 6.25a1.75 1.75 0 0 1 0 2.474l-5.026 5.026a1.75 1.75 0 0 1-2.474 0l-6.25-6.25A1.752 1.752 0 0 1 1 7.775Zm1.5 0c0 .066.026.13.073.177l6.25 6.25a.25.25 0 0 0 .354 0l5.025-5.025a.25.25 0 0 0 0-.354l-6.25-6.25a.25.25 0 0 0-.177-.073H2.75a.25.25 0 0 0-.25.25ZM6 5a1 1 0 1 1 0 2 1 1 0 0 1 0-2Z";
const GIT_COMMIT_16: &str = "M11.93 8.5a4.002 4.002 0 0 1-7.86 0H.75a.75.75 0 0 1 0-1.5h3.32a4.002 4.002 0 0 1 7.86 0h3.32a.75.75 0 0 1 0 1.5Zm-1.43-.75a2.5 2.5 0 1 0-5 0 2.5 2.5 0 0 0 5 0Z";
const REPO_16: &str = "M2 2.5A2.5 2.5 0 0 1 4.5 0h8.75a.75.75 0 0 1 .75.75v12.5a.75.75 0 0 1-.75.75h-2.5a.75.75 0 0 1 0-1.5h1.75v-2h-8a1 1 0 0 0-.714 1.7.75.75 0 1 1-1.072 1.05A2.495 2.495 0 0 1 2 11.5Zm10.5-1h-8a1 1 0 0 0-1 1v6.708A2.486 2.486 0 0 1 4.5 9h8ZM5 12.25a.25.25 0 0 1 .25-.25h3.5a.25.25 0 0 1 .25.25v3.25a.25.25 0 0 1-.4.2l-1.45-1.087a.249.249 0 0 0-.3 0L5.4 15.7a.25.25 0 0 1-.4-.2Z";
const REPO_PUSH_16: &str = "M2 2.5A2.5 2.5 0 0 1 4.5 0h8.75a.75.75 0 0 1 .75.75v3.5a.75.75 0 0 1-1.5 0V1.5h-8a1 1 0 0 0-1 1v6.708A2.493 2.493 0 0 1 4.5 9h2.25a.75.75 0 0 1 0 1.5H4.5a1 1 0 0 0 0 2h4.75a.75.75 0 0 1 0 1.5H4.5A2.5 2.5 0 0 1 2 11.5Zm12.23 7.79h-.001l-1.224-1.224v6.184a.75.75 0 0 1-1.5 0V9.066L10.28 10.29a.75.75 0 0 1-1.06-1.061l2.505-2.504a.75.75 0 0 1 1.06 0L15.29 9.23a.751.751 0 0 1-.018 1.042.751.751 0 0 1-1.042.018Z";
const STACK_16: &str = "M7.122.392a1.75 1.75 0 0 1 1.756 0l5.003 2.902c.83.481.83 1.68 0 2.162L8.878 8.358a1.75 1.75 0 0 1-1.756 0L2.119 5.456a1.251 1.251 0 0 1 0-2.162ZM8.125 1.69a.248.248 0 0 0-.25 0l-4.63 2.685 4.63 2.685a.248.248 0 0 0 .25 0l4.63-2.685ZM1.601 7.789a.75.75 0 0 1 1.025-.273l5.249 3.044a.248.248 0 0 0 .25 0l5.249-3.044a.75.75 0 0 1 .752 1.298l-5.248 3.044a1.75 1.75 0 0 1-1.756 0L1.874 8.814A.75.75 0 0 1 1.6 7.789Zm0 3.5a.75.75 0 0 1 1.025-.273l5.249 3.044a.248.248 0 0 0 .25 0l5.249-3.044a.75.75 0 0 1 .752 1.298l-5.248 3.044a1.75 1.75 0 0 1-1.756 0l-5.248-3.044a.75.75 0 0 1-.273-1.025Z";
const FILE_DIRECTORY_16: &str = "M0 2.75C0 1.784.784 1 1.75 1H5c.55 0 1.07.26 1.4.7l.9 1.2a.25.25 0 0 0 .2.1h6.75c.966 0 1.75.784 1.75 1.75v8.5A1.75 1.75 0 0 1 14.25 15H1.75A1.75 1.75 0 0 1 0 13.25Zm1.75-.25a.25.25 0 0 0-.25.25v10.5c0 .138.112.25.25.25h12.5a.25.25 0 0 0 .25-.25v-8.5a.25.25 0 0 0-.25-.25H7.5c-.55 0-1.07-.26-1.4-.7l-.9-1.2a.25.25 0 0 0-.2-.1Z";
const WORKFLOW_16: &str = "M0 1.75C0 .784.784 0 1.75 0h3.5C6.216 0 7 .784 7 1.75v3.5A1.75 1.75 0 0 1 5.25 7H4v4a1 1 0 0 0 1 1h4v-1.25C9 9.784 9.784 9 10.75 9h3.5c.966 0 1.75.784 1.75 1.75v3.5A1.75 1.75 0 0 1 14.25 16h-3.5A1.75 1.75 0 0 1 9 14.25v-.75H5A2.5 2.5 0 0 1 2.5 11V7h-.75A1.75 1.75 0 0 1 0 5.25Zm1.75-.25a.25.25 0 0 0-.25.25v3.5c0 .138.112.25.25.25h3.5a.25.25 0 0 0 .25-.25v-3.5a.25.25 0 0 0-.25-.25Zm9 9a.25.25 0 0 0-.25.25v3.5c0 .138.112.25.25.25h3.5a.25.25 0 0 0 .25-.25v-3.5a.25.25 0 0 0-.25-.25Z";
const ALERT_16: &str = "M6.457 1.047c.659-1.234 2.427-1.234 3.086 0l6.082 11.378A1.75 1.75 0 0 1 14.082 15H1.918a1.75 1.75 0 0 1-1.543-2.575Zm1.763.707a.25.25 0 0 0-.44 0L1.698 13.132a.25.25 0 0 0 .22.368h12.164a.25.25 0 0 0 .22-.368Zm.53 3.996v2.5a.75.75 0 0 1-1.5 0v-2.5a.75.75 0 0 1 1.5 0ZM9 11a1 1 0 1 1-2 0 1 1 0 0 1 2 0Z";
const SEARCH_16: &str = "M10.68 11.74a6 6 0 0 1-7.922-8.982 6 6 0 0 1 8.982 7.922l3.04 3.04a.749.749 0 0 1-.326 1.275.749.749 0 0 1-.734-.215ZM11.5 7a4.499 4.499 0 1 0-8.997 0A4.499 4.499 0 0 0 11.5 7Z";
const HISTORY_16: &str = "m.427 1.927 1.215 1.215a8.002 8.002 0 1 1-1.6 5.685.75.75 0 1 1 1.493-.154 6.5 6.5 0 1 0 1.18-4.458l1.358 1.358A.25.25 0 0 1 3.896 6H.25A.25.25 0 0 1 0 5.75V2.104a.25.25 0 0 1 .427-.177ZM7.75 4a.75.75 0 0 1 .75.75v2.992l2.028.812a.75.75 0 0 1-.557 1.392l-2.5-1A.751.751 0 0 1 7 8.25v-3.5A.75.75 0 0 1 7.75 4Z";
const FILE_SUBMODULE_16: &str = "M0 2.75C0 1.784.784 1 1.75 1H5c.55 0 1.07.26 1.4.7l.9 1.2a.25.25 0 0 0 .2.1h6.75c.966 0 1.75.784 1.75 1.75v8.5A1.75 1.75 0 0 1 14.25 15H1.75A1.75 1.75 0 0 1 0 13.25Zm9.42 9.36 2.883-2.677a.25.25 0 0 0 0-.366L9.42 6.39a.249.249 0 0 0-.42.183V8.5H4.75a.75.75 0 0 0 0 1.5H9v1.927c0 .218.26.331.42.183Z";
// Note editor toolbar (octicons 19.42.0, verbatim).
const BOLD_16: &str = "M4 2h4.5a3.501 3.501 0 0 1 2.852 5.53A3.499 3.499 0 0 1 9.5 14H4a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1Zm1 7v3h4.5a1.5 1.5 0 0 0 0-3Zm3.5-2a1.5 1.5 0 0 0 0-3H5v3Z";
const ITALIC_16: &str = "M6 2.75A.75.75 0 0 1 6.75 2h6.5a.75.75 0 0 1 0 1.5h-2.505l-3.858 9H9.25a.75.75 0 0 1 0 1.5h-6.5a.75.75 0 0 1 0-1.5h2.505l3.858-9H6.75A.75.75 0 0 1 6 2.75Z";
const CODE_16: &str = "m11.28 3.22 4.25 4.25a.75.75 0 0 1 0 1.06l-4.25 4.25a.749.749 0 0 1-1.275-.326.749.749 0 0 1 .215-.734L13.94 8l-3.72-3.72a.749.749 0 0 1 .326-1.275.749.749 0 0 1 .734.215Zm-6.56 0a.751.751 0 0 1 1.042.018.751.751 0 0 1 .018 1.042L2.06 8l3.72 3.72a.749.749 0 0 1-.326 1.275.749.749 0 0 1-.734-.215L.47 8.53a.75.75 0 0 1 0-1.06Z";
const LINK_16: &str = "m7.775 3.275 1.25-1.25a3.5 3.5 0 1 1 4.95 4.95l-2.5 2.5a3.5 3.5 0 0 1-4.95 0 .751.751 0 0 1 .018-1.042.751.751 0 0 1 1.042-.018 1.998 1.998 0 0 0 2.83 0l2.5-2.5a2.002 2.002 0 0 0-2.83-2.83l-1.25 1.25a.751.751 0 0 1-1.042-.018.751.751 0 0 1-.018-1.042Zm-4.69 9.64a1.998 1.998 0 0 0 2.83 0l1.25-1.25a.751.751 0 0 1 1.042.018.751.751 0 0 1 .018 1.042l-1.25 1.25a3.5 3.5 0 1 1-4.95-4.95l2.5-2.5a3.5 3.5 0 0 1 4.95 0 .751.751 0 0 1-.018 1.042.751.751 0 0 1-1.042.018 1.998 1.998 0 0 0-2.83 0l-2.5 2.5a1.998 1.998 0 0 0 0 2.83Z";
const HEADING_16: &str = "M3.75 2a.75.75 0 0 1 .75.75V7h7V2.75a.75.75 0 0 1 1.5 0v10.5a.75.75 0 0 1-1.5 0V8.5h-7v4.75a.75.75 0 0 1-1.5 0V2.75A.75.75 0 0 1 3.75 2Z";
const LIST_UNORDERED_16: &str = "M5.75 2.5h8.5a.75.75 0 0 1 0 1.5h-8.5a.75.75 0 0 1 0-1.5Zm0 5h8.5a.75.75 0 0 1 0 1.5h-8.5a.75.75 0 0 1 0-1.5Zm0 5h8.5a.75.75 0 0 1 0 1.5h-8.5a.75.75 0 0 1 0-1.5ZM2 14a1 1 0 1 1 0-2 1 1 0 0 1 0 2Zm1-6a1 1 0 1 1-2 0 1 1 0 0 1 2 0ZM2 4a1 1 0 1 1 0-2 1 1 0 0 1 0 2Z";
const TASKLIST_16: &str = "M2 2h4a1 1 0 0 1 1 1v4a1 1 0 0 1-1 1H2a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1Zm4.655 8.595a.75.75 0 0 1 0 1.06L4.03 14.28a.75.75 0 0 1-1.06 0l-1.5-1.5a.749.749 0 0 1 .326-1.275.749.749 0 0 1 .734.215l.97.97 2.095-2.095a.75.75 0 0 1 1.06 0ZM9.75 2.5h5.5a.75.75 0 0 1 0 1.5h-5.5a.75.75 0 0 1 0-1.5Zm0 5h5.5a.75.75 0 0 1 0 1.5h-5.5a.75.75 0 0 1 0-1.5Zm0 5h5.5a.75.75 0 0 1 0 1.5h-5.5a.75.75 0 0 1 0-1.5Zm-7.25-9v3h3v-3Z";
const QUOTE_16: &str = "M1.75 2.5h10.5a.75.75 0 0 1 0 1.5H1.75a.75.75 0 0 1 0-1.5Zm4 5h8.5a.75.75 0 0 1 0 1.5h-8.5a.75.75 0 0 1 0-1.5Zm0 5h8.5a.75.75 0 0 1 0 1.5h-8.5a.75.75 0 0 1 0-1.5ZM2.5 7.75v6a.75.75 0 0 1-1.5 0v-6a.75.75 0 0 1 1.5 0Z";
const KEBAB_HORIZONTAL_16: &str = "M8 9a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3ZM1.5 9a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3Zm13 0a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3Z";
// File status in diffs (octicons 19.42.0, verbatim).
const DIFF_ADDED_16: &str = "M2.75 1h10.5c.966 0 1.75.784 1.75 1.75v10.5A1.75 1.75 0 0 1 13.25 15H2.75A1.75 1.75 0 0 1 1 13.25V2.75C1 1.784 1.784 1 2.75 1Zm10.5 1.5H2.75a.25.25 0 0 0-.25.25v10.5c0 .138.112.25.25.25h10.5a.25.25 0 0 0 .25-.25V2.75a.25.25 0 0 0-.25-.25ZM8 4a.75.75 0 0 1 .75.75v2.5h2.5a.75.75 0 0 1 0 1.5h-2.5v2.5a.75.75 0 0 1-1.5 0v-2.5h-2.5a.75.75 0 0 1 0-1.5h2.5v-2.5A.75.75 0 0 1 8 4Z";
const DIFF_REMOVED_16: &str = "M13.25 1c.966 0 1.75.784 1.75 1.75v10.5A1.75 1.75 0 0 1 13.25 15H2.75A1.75 1.75 0 0 1 1 13.25V2.75C1 1.784 1.784 1 2.75 1ZM2.75 2.5a.25.25 0 0 0-.25.25v10.5c0 .138.112.25.25.25h10.5a.25.25 0 0 0 .25-.25V2.75a.25.25 0 0 0-.25-.25Zm8.5 6.25h-6.5a.75.75 0 0 1 0-1.5h6.5a.75.75 0 0 1 0 1.5Z";
const DIFF_RENAMED_16: &str = "M13.25 1c.966 0 1.75.784 1.75 1.75v10.5A1.75 1.75 0 0 1 13.25 15H2.75A1.75 1.75 0 0 1 1 13.25V2.75C1 1.784 1.784 1 2.75 1ZM2.75 2.5a.25.25 0 0 0-.25.25v10.5c0 .138.112.25.25.25h10.5a.25.25 0 0 0 .25-.25V2.75a.25.25 0 0 0-.25-.25Zm9.03 6.03-3.25 3.25a.749.749 0 0 1-1.275-.326.749.749 0 0 1 .215-.734l1.97-1.97H4.75a.75.75 0 0 1 0-1.5h4.69L7.47 5.28a.751.751 0 0 1 .018-1.042.751.751 0 0 1 1.042-.018l3.25 3.25a.75.75 0 0 1 0 1.06Z";

fn source(icon: Icon, px: usize) -> (&'static str, f32) {
    // Octicons ship hand-tuned 16 and 24 grids; use the closer one.
    match (icon, px <= 20) {
        (Icon::Github, true) => (GITHUB_16, 16.0),
        (Icon::Github, false) => (GITHUB_24, 24.0),
        (Icon::Note, true) => (NOTE_16, 16.0),
        (Icon::Note, false) => (NOTE_24, 24.0),
        // Group icons: 16px grid only (shown at ~14px).
        (Icon::GitBranch, _) => (GIT_BRANCH_16, 16.0),
        (Icon::Tag, _) => (TAG_16, 16.0),
        (Icon::GitCommit, _) => (GIT_COMMIT_16, 16.0),
        (Icon::Repo, _) => (REPO_16, 16.0),
        (Icon::RepoPush, _) => (REPO_PUSH_16, 16.0),
        (Icon::Stack, _) => (STACK_16, 16.0),
        (Icon::FileDirectory, _) => (FILE_DIRECTORY_16, 16.0),
        (Icon::Workflow, _) => (WORKFLOW_16, 16.0),
        (Icon::Alert, _) => (ALERT_16, 16.0),
        (Icon::Search, _) => (SEARCH_16, 16.0),
        (Icon::FileSubmodule, _) => (FILE_SUBMODULE_16, 16.0),
        (Icon::History, _) => (HISTORY_16, 16.0),
        (Icon::Bold, _) => (BOLD_16, 16.0),
        (Icon::Italic, _) => (ITALIC_16, 16.0),
        (Icon::Code, _) => (CODE_16, 16.0),
        (Icon::Link, _) => (LINK_16, 16.0),
        (Icon::Heading, _) => (HEADING_16, 16.0),
        (Icon::ListUnordered, _) => (LIST_UNORDERED_16, 16.0),
        (Icon::Tasklist, _) => (TASKLIST_16, 16.0),
        (Icon::Quote, _) => (QUOTE_16, 16.0),
        (Icon::KebabHorizontal, _) => (KEBAB_HORIZONTAL_16, 16.0),
        (Icon::DiffAdded, _) => (DIFF_ADDED_16, 16.0),
        (Icon::DiffRemoved, _) => (DIFF_REMOVED_16, 16.0),
        (Icon::DiffRenamed, _) => (DIFF_RENAMED_16, 16.0),
    }
}

#[derive(Clone, Default)]
struct Cache(HashMap<(Icon, usize), TextureHandle>);

/// Texture for `icon` drawn at `size` points (square).
pub fn texture(ctx: &egui::Context, icon: Icon, size: f32) -> SizedTexture {
    let px = (size * ctx.pixels_per_point()).round().max(4.0) as usize;
    let id = egui::Id::unique("margin_icon_cache");
    let cached = ctx.memory_mut(|m| m.data.get_temp_mut_or_default::<Cache>(id).0.get(&(icon, px)).cloned());
    let tex = match cached {
        Some(t) => t,
        None => {
            let (d, view) = source(icon, px);
            let alpha = rasterize(d, view, px);
            let img = ColorImage::new([px, px], alpha.into_iter().map(Color32::from_white_alpha).collect());
            let t = ctx.load_texture(format!("icon-{icon:?}-{px}"), img, TextureOptions::LINEAR);
            ctx.memory_mut(|m| m.data.get_temp_mut_or_default::<Cache>(id).0.insert((icon, px), t.clone()));
            t
        }
    };
    SizedTexture::new(tex.id(), vec2(size, size))
}

/// Paint `icon` centred in `rect` (square of the smaller side), tinted.
pub fn paint(p: &egui::Painter, icon: Icon, rect: Rect, tint: Color32) {
    let s = rect.width().min(rect.height());
    let r = Rect::from_center_size(rect.center(), vec2(s, s));
    let tex = texture(p.ctx(), icon, s);
    p.image(tex.id, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
}

/// An `egui::Image` widget/atom for labels, tabs and buttons.
pub fn image(ctx: &egui::Context, icon: Icon, size: f32, tint: Color32) -> egui::Image<'static> {
    egui::Image::from_texture(texture(ctx, icon, size)).tint(tint)
}

// ------------------------------------------------------------- rasteriser

/// Fill an SVG path (nonzero rule) into a `px`×`px` alpha mask, 4×4 supersampled.
pub fn rasterize(d: &str, view: f32, px: usize) -> Vec<u8> {
    const SS: usize = 4;
    let scale = px as f32 / view;
    let polys: Vec<Vec<(f32, f32)>> = flatten(d)
        .into_iter()
        .map(|p| p.into_iter().map(|(x, y)| (x * scale, y * scale)).collect())
        .collect();
    let mut cov = vec![0u16; px * px];
    let mut xs: Vec<(f32, i32)> = Vec::new();
    for row in 0..px {
        for sub in 0..SS {
            let y = row as f32 + (sub as f32 + 0.5) / SS as f32;
            xs.clear();
            for poly in &polys {
                for i in 0..poly.len() {
                    let (x0, y0) = poly[i];
                    let (x1, y1) = poly[(i + 1) % poly.len()];
                    if (y0 <= y && y1 > y) || (y1 <= y && y0 > y) {
                        let t = (y - y0) / (y1 - y0);
                        xs.push((x0 + t * (x1 - x0), if y1 > y0 { 1 } else { -1 }));
                    }
                }
            }
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut wind = 0;
            for w in xs.windows(2) {
                wind += w[0].1;
                if wind == 0 {
                    continue;
                }
                let (a, b) = (w[0].0.max(0.0), w[1].0.min(px as f32));
                // count horizontal sample points (c + (j+.5)/SS) inside [a, b)
                let first = ((a * SS as f32) - 0.5).ceil().max(0.0) as usize;
                let last = ((b * SS as f32) - 0.5).ceil().max(0.0) as usize;
                for s in first..last.min(px * SS) {
                    cov[row * px + s / SS] += 1;
                }
            }
        }
    }
    cov.into_iter().map(|c| ((c as u32 * 255) / (SS * SS) as u32) as u8).collect()
}

/// Parse SVG path data into closed polylines (curves and arcs flattened).
fn flatten(d: &str) -> Vec<Vec<(f32, f32)>> {
    let mut t = Tok { s: d.as_bytes(), i: 0 };
    let mut polys: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0, 0.0);
    let mut last_ctrl: Option<(f32, f32)> = None; // for S/T reflection
    let mut cmd = b'M';
    let close = |cur: &mut Vec<(f32, f32)>, polys: &mut Vec<Vec<(f32, f32)>>| {
        if cur.len() > 2 {
            polys.push(std::mem::take(cur));
        } else {
            cur.clear();
        }
    };
    loop {
        t.skip_ws();
        if t.i >= t.s.len() {
            break;
        }
        let c = t.s[t.i];
        if c.is_ascii_alphabetic() {
            cmd = c;
            t.i += 1;
        } else if cmd == b'M' || cmd == b'm' {
            // implicit lineto after moveto
            cmd = if cmd == b'M' { b'L' } else { b'l' };
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (x, y) } else { (0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                close(&mut cur, &mut polys);
                let (nx, ny) = (t.num() + ox, t.num() + oy);
                x = nx;
                y = ny;
                sx = x;
                sy = y;
                cur.push((x, y));
                last_ctrl = None;
            }
            b'L' => {
                x = t.num() + ox;
                y = t.num() + oy;
                cur.push((x, y));
                last_ctrl = None;
            }
            b'H' => {
                x = t.num() + if rel { x } else { 0.0 };
                cur.push((x, y));
                last_ctrl = None;
            }
            b'V' => {
                y = t.num() + if rel { y } else { 0.0 };
                cur.push((x, y));
                last_ctrl = None;
            }
            b'C' | b'S' => {
                let (c1x, c1y) = if cmd.to_ascii_uppercase() == b'C' {
                    (t.num() + ox, t.num() + oy)
                } else {
                    last_ctrl.map(|(cx, cy)| (2.0 * x - cx, 2.0 * y - cy)).unwrap_or((x, y))
                };
                let (c2x, c2y) = (t.num() + ox, t.num() + oy);
                let (ex, ey) = (t.num() + ox, t.num() + oy);
                for k in 1..=12 {
                    let u = k as f32 / 12.0;
                    let v = 1.0 - u;
                    cur.push((
                        v * v * v * x + 3.0 * v * v * u * c1x + 3.0 * v * u * u * c2x + u * u * u * ex,
                        v * v * v * y + 3.0 * v * v * u * c1y + 3.0 * v * u * u * c2y + u * u * u * ey,
                    ));
                }
                last_ctrl = Some((c2x, c2y));
                x = ex;
                y = ey;
            }
            b'Q' | b'T' => {
                let (cx, cy) = if cmd.to_ascii_uppercase() == b'Q' {
                    (t.num() + ox, t.num() + oy)
                } else {
                    last_ctrl.map(|(cx, cy)| (2.0 * x - cx, 2.0 * y - cy)).unwrap_or((x, y))
                };
                let (ex, ey) = (t.num() + ox, t.num() + oy);
                for k in 1..=10 {
                    let u = k as f32 / 10.0;
                    let v = 1.0 - u;
                    cur.push((v * v * x + 2.0 * v * u * cx + u * u * ex, v * v * y + 2.0 * v * u * cy + u * u * ey));
                }
                last_ctrl = Some((cx, cy));
                x = ex;
                y = ey;
            }
            b'A' => {
                let (rx, ry, rot) = (t.num(), t.num(), t.num());
                let (large, sweep) = (t.flag(), t.flag());
                let (ex, ey) = (t.num() + ox, t.num() + oy);
                arc(&mut cur, (x, y), (ex, ey), rx, ry, rot, large, sweep);
                x = ex;
                y = ey;
                last_ctrl = None;
            }
            b'Z' => {
                close(&mut cur, &mut polys);
                x = sx;
                y = sy;
                cur.push((x, y));
                last_ctrl = None;
                // A command letter must follow Z; avoid looping on stray numbers.
                t.skip_ws();
                if t.i < t.s.len() && !t.s[t.i].is_ascii_alphabetic() {
                    t.num();
                }
            }
            _ => {
                t.i += 1; // unknown: skip
            }
        }
    }
    close(&mut cur, &mut polys);
    polys
}

/// SVG endpoint arc → points (spec F.6.5).
#[allow(clippy::too_many_arguments)]
fn arc(out: &mut Vec<(f32, f32)>, p0: (f32, f32), p1: (f32, f32), rx: f32, ry: f32, rot_deg: f32, large: bool, sweep: bool) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 || p0 == p1 {
        out.push(p1);
        return;
    }
    let phi = rot_deg.to_radians();
    let (cs, sn) = (phi.cos(), phi.sin());
    let dx = (p0.0 - p1.0) / 2.0;
    let dy = (p0.1 - p1.1) / 2.0;
    let x1 = cs * dx + sn * dy;
    let y1 = -sn * dx + cs * dy;
    let lam = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lam > 1.0 {
        rx *= lam.sqrt();
        ry *= lam.sqrt();
    }
    let num = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut co = (num / den).sqrt();
    if large == sweep {
        co = -co;
    }
    let cxp = co * rx * y1 / ry;
    let cyp = -co * ry * x1 / rx;
    let cx = cs * cxp - sn * cyp + (p0.0 + p1.0) / 2.0;
    let cy = sn * cxp + cs * cyp + (p0.1 + p1.1) / 2.0;
    let ang = |ux: f32, uy: f32, vx: f32, vy: f32| {
        let a = (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
        a
    };
    let t1 = ang(1.0, 0.0, (x1 - cxp) / rx, (y1 - cyp) / ry);
    let mut dt = ang((x1 - cxp) / rx, (y1 - cyp) / ry, (-x1 - cxp) / rx, (-y1 - cyp) / ry);
    if !sweep && dt > 0.0 {
        dt -= std::f32::consts::TAU;
    } else if sweep && dt < 0.0 {
        dt += std::f32::consts::TAU;
    }
    let n = ((dt.abs() / (std::f32::consts::PI / 16.0)).ceil() as usize).max(2);
    for k in 1..=n {
        let a = t1 + dt * k as f32 / n as f32;
        let (ca, sa) = (a.cos(), a.sin());
        out.push((cx + cs * rx * ca - sn * ry * sa, cy + sn * rx * ca + cs * ry * sa));
    }
}

struct Tok<'a> {
    s: &'a [u8],
    i: usize,
}

impl Tok<'_> {
    fn skip_ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',') {
            self.i += 1;
        }
    }
    fn flag(&mut self) -> bool {
        self.skip_ws();
        let f = self.s.get(self.i) == Some(&b'1');
        self.i += 1;
        f
    }
    fn num(&mut self) -> f32 {
        self.skip_ws();
        let start = self.i;
        let s = self.s;
        if self.i < s.len() && (s[self.i] == b'-' || s[self.i] == b'+') {
            self.i += 1;
        }
        let mut dot = false;
        while self.i < s.len() {
            match s[self.i] {
                b'0'..=b'9' => {}
                b'.' if !dot => dot = true,
                b'e' | b'E' => {
                    self.i += 1;
                    if self.i < s.len() && (s[self.i] == b'-' || s[self.i] == b'+') {
                        self.i += 1;
                    }
                    continue;
                }
                _ => break,
            }
            self.i += 1;
        }
        std::str::from_utf8(&s[start..self.i]).ok().and_then(|t| t.parse().ok()).unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_icon_shape() {
        // 16px note: frame + hollow inside + two text lines.
        let a = rasterize(NOTE_16, 16.0, 16);
        let at = |x: usize, y: usize| a[y * 16 + x];
        assert!(at(0, 8) > 200, "left frame edge is filled");
        assert!(at(2, 4) < 40, "inside the frame is empty");
        assert!(at(6, 6) > 200, "first text line");
        assert!(at(12, 9) < 40, "second line is shorter");
        assert!(at(8, 0) < 10 && at(8, 15) < 10, "nothing above/below the page");
    }

    #[test]
    fn all_group_icons_rasterize() {
        for i in [Icon::GitBranch, Icon::Tag, Icon::GitCommit, Icon::Repo, Icon::RepoPush, Icon::Stack, Icon::FileDirectory,
                  Icon::Workflow, Icon::Alert, Icon::Search, Icon::FileSubmodule] {
            let (d, v) = source(i, 32);
            let a = rasterize(d, v, 32);
            let ink = a.iter().filter(|&&x| x > 128).count();
            assert!((40..900).contains(&ink), "{i:?}: implausible coverage {ink}");
        }
    }

    #[test]
    fn github_mark_shape() {
        let a = rasterize(GITHUB_24, 24.0, 48);
        let at = |x: usize, y: usize| a[y * 48 + x];
        assert!(at(24, 5) > 200, "top of the circle ring");
        assert!(at(24, 20) < 40, "the cat face is cut out");
        assert!(at(2, 2) < 10, "outside the circle");
        let filled = a.iter().filter(|&&v| v > 128).count();
        assert!((600..1400).contains(&filled), "plausible ink coverage: {filled}");
    }
}

#[cfg(test)]
#[test]
#[ignore]
fn dump_ascii() {
    for i in [Icon::GitBranch, Icon::FileSubmodule, Icon::Workflow, Icon::Tag] {
        let (d, v) = source(i, 16);
        let a = rasterize(d, v, 16);
        println!("{i:?}");
        for y in 0..16 {
            println!("{}", (0..16).map(|x| match a[y * 16 + x] { 0..=40 => ' ', 41..=160 => '+', _ => '#' }).collect::<String>());
        }
    }
}
