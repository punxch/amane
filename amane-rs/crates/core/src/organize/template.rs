//! 路径模板: 占位符替换、空字段折叠、按 Windows 创建结果规范化每个路径段.

use std::collections::BTreeMap;

use thiserror::Error;

/// 单个路径段的字节上限. 与 Python 版 `PATH_FIELD_MAX_BYTES` 对齐, 省略号计入上限.
pub const MAX_SEGMENT_BYTES: usize = 200;
const ELLIPSIS: &str = "…";

/// 允许出现在模板里的占位符. 白名单在解析期生效, 拼错的变量立即报错而不是渲染成 Unknown.
pub const PLACEHOLDERS: &[&str] = &[
    "number",
    "prefix",
    "suffix",
    "title",
    "maker",
    "label",
    "series",
    "year",
    "release",
    "content_type",
    "ext",
    "raw_dir",
    "raw_name",
];

/// 长字段写入路径前按字节截断; 与 Python 版的 `_CLIP_KEYS` 同集合 (Rust 切片无演员变量).
const CLIP_KEYS: &[&str] = &["title", "maker", "label", "series"];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TemplateError {
    #[error("占位符未闭合")]
    UnclosedPlaceholder,
    #[error("可选组未闭合")]
    UnclosedGroup,
    #[error("] 没有匹配的 [")]
    StrayBracket,
    #[error("存在空的占位符")]
    EmptyPlaceholder,
    #[error("占位符里嵌套了花括号")]
    NestedBrace,
    #[error("未知占位符: {0}")]
    UnknownPlaceholder(String),
    #[error("占位符映射无效: {0}")]
    BadMapping(String),
    #[error("映射键重复: {0}")]
    DuplicateMappingKey(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Literal(String),
    Placeholder {
        name: String,
        mapping: Vec<(String, String)>,
    },
    Group {
        children: Vec<Node>,
        /// `[[...]]`: 有值时把结果包一层 `[]`.
        wrap: bool,
    },
}

/// 解析后的模板. 可选组 `[...]` 内**直接**占位符全部为空串时整组丢弃, 有一个非空则渲染.
#[derive(Debug, Clone)]
pub struct Template {
    source: String,
    nodes: Vec<Node>,
}

impl Template {
    /// 解析并校验占位符名.
    pub fn parse(source: &str) -> Result<Self, TemplateError> {
        let mut parser = Parser {
            src: source,
            position: 0,
        };
        let nodes = parser.parse_nodes(None)?;
        Ok(Self {
            source: source.to_owned(),
            nodes,
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// 渲染: 长字段截断、组折叠、按 Windows 创建结果规范化每个路径段.
    pub fn render(&self, variables: &BTreeMap<String, String>) -> String {
        let clipped: BTreeMap<String, String> = variables
            .iter()
            .map(|(name, value)| {
                let value = if CLIP_KEYS.contains(&name.as_str()) {
                    clip_bytes(value, MAX_SEGMENT_BYTES)
                } else {
                    value.clone()
                };
                (name.clone(), value)
            })
            .collect();
        let filled = render_nodes(&self.nodes, &clipped);
        normalize_segments(&self.source, &filled)
    }
}

struct Parser<'a> {
    src: &'a str,
    position: usize,
}

impl<'a> Parser<'a> {
    fn parse_nodes(&mut self, closer: Option<&str>) -> Result<Vec<Node>, TemplateError> {
        let mut nodes = Vec::new();
        let mut buffer = String::new();

        while self.position < self.src.len() {
            let rest = &self.src[self.position..];
            if let Some(closing) = closer
                && rest.starts_with(closing)
            {
                self.position += closing.len();
                if !buffer.is_empty() {
                    nodes.push(Node::Literal(buffer));
                }
                return Ok(nodes);
            }
            if rest.starts_with("[[") {
                if !buffer.is_empty() {
                    nodes.push(Node::Literal(buffer.clone()));
                    buffer.clear();
                }
                self.position += 2;
                let children = self.parse_nodes(Some("]]"))?;
                nodes.push(Node::Group {
                    children,
                    wrap: true,
                });
                continue;
            }
            let first = rest.chars().next().expect("非空串必有字符");
            match first {
                '[' => {
                    if !buffer.is_empty() {
                        nodes.push(Node::Literal(buffer.clone()));
                        buffer.clear();
                    }
                    self.position += 1;
                    let children = self.parse_nodes(Some("]"))?;
                    nodes.push(Node::Group {
                        children,
                        wrap: false,
                    });
                }
                '{' => {
                    if !buffer.is_empty() {
                        nodes.push(Node::Literal(buffer.clone()));
                        buffer.clear();
                    }
                    let placeholder = self.parse_placeholder()?;
                    nodes.push(placeholder);
                }
                ']' => return Err(TemplateError::StrayBracket),
                _ => {
                    buffer.push(first);
                    self.position += first.len_utf8();
                }
            }
        }

        if !buffer.is_empty() {
            nodes.push(Node::Literal(buffer));
        }
        if closer.is_some() {
            return Err(TemplateError::UnclosedGroup);
        }
        Ok(nodes)
    }

    fn parse_placeholder(&mut self) -> Result<Node, TemplateError> {
        self.position += 1; // 跳过 '{'
        let start = self.position;
        let bytes = self.src.as_bytes();
        while self.position < bytes.len() && bytes[self.position] != b'}' {
            if bytes[self.position] == b'{' {
                return Err(TemplateError::NestedBrace);
            }
            self.position += 1;
        }
        if self.position >= bytes.len() {
            return Err(TemplateError::UnclosedPlaceholder);
        }
        let body = &self.src[start..self.position];
        self.position += 1; // 跳过 '}'

        let (name_part, map_part) = match body.split_once('|') {
            Some((name, map)) => (name, Some(map)),
            None => (body, None),
        };
        let name = name_part.trim();
        if name.is_empty() {
            return Err(TemplateError::EmptyPlaceholder);
        }
        if !PLACEHOLDERS.contains(&name) {
            return Err(TemplateError::UnknownPlaceholder(name.to_owned()));
        }
        let mapping = match map_part {
            Some(spec) => parse_mapping(name, spec)?,
            None => Vec::new(),
        };
        Ok(Node::Placeholder {
            name: name.to_owned(),
            mapping,
        })
    }
}

/// `{name|k=v,k=v}` 的映射: `k=` 允许 (空源 → 缺省), 键不得重复; 空映射体非法.
fn parse_mapping(name: &str, spec: &str) -> Result<Vec<(String, String)>, TemplateError> {
    if spec.trim().is_empty() {
        return Err(TemplateError::BadMapping(name.to_owned()));
    }
    let mut pairs = Vec::new();
    let mut seen = Vec::new();
    for item in spec.split(',') {
        let Some((key, value)) = item.split_once('=') else {
            return Err(TemplateError::BadMapping(name.to_owned()));
        };
        let key = key.trim();
        if seen.contains(&key) {
            return Err(TemplateError::DuplicateMappingKey(key.to_owned()));
        }
        seen.push(key);
        pairs.push((key.to_owned(), value.trim().to_owned()));
    }
    Ok(pairs)
}

fn lookup<'a>(variables: &'a BTreeMap<String, String>, name: &str) -> &'a str {
    variables.get(name).map(String::as_str).unwrap_or_default()
}

/// 查值后按映射改写; 空源值保持空串, 可选组据此折叠.
fn resolve(node: &Node, variables: &BTreeMap<String, String>) -> Option<String> {
    match node {
        Node::Literal(text) => Some(text.clone()),
        Node::Placeholder { name, mapping } => {
            let raw = lookup(variables, name);
            let value = mapping
                .iter()
                .find(|(key, _)| key == raw)
                .map(|(_, value)| value.clone())
                .unwrap_or_else(|| raw.to_owned());
            Some(value)
        }
        Node::Group { children, wrap } => {
            let direct: Vec<&Node> = children
                .iter()
                .filter(|child| matches!(child, Node::Placeholder { .. }))
                .collect();
            // 组内直接占位符全空则整组丢弃; 嵌套组各自判断, 不并入外层.
            if !direct.is_empty()
                && direct.iter().all(
                    |child| matches!(resolve(child, variables), Some(value) if value.is_empty()),
                )
            {
                return None;
            }
            let inner: String = children
                .iter()
                .filter_map(|child| resolve(child, variables))
                .collect();
            Some(if *wrap { format!("[{inner}]") } else { inner })
        }
    }
}

fn render_nodes(nodes: &[Node], variables: &BTreeMap<String, String>) -> String {
    nodes
        .iter()
        .filter_map(|node| resolve(node, variables))
        .collect()
}

/// 元数据字段写入路径段前的清洗: 控制字符与目录分隔符替换为空格, Windows 保留字符剔除.
pub fn sanitize_field(value: &str) -> String {
    let mut cleaned = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            c if c.is_control() || c == '/' || c == '\\' || c == ':' => cleaned.push(' '),
            '*' | '?' | '"' | '<' | '>' | '|' => {}
            _ => cleaned.push(character),
        }
    }
    cleaned.trim().to_owned()
}

/// 按字节截断并追加省略号, 不切开多字节字符; 省略号计入上限.
pub fn clip_bytes(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let budget = limit.saturating_sub(ELLIPSIS.len());
    let mut clipped = &value.as_bytes()[..budget.min(value.len())];
    while !clipped.is_empty() {
        if let Ok(text) = std::str::from_utf8(clipped) {
            return format!("{text}{ELLIPSIS}");
        }
        clipped = &clipped[..clipped.len() - 1];
    }
    ELLIPSIS.to_owned()
}

/// 路径段字面值: 控制字符替换为空格, 去掉段尾的 `.` 与空格.
///
/// Windows 创建时静默丢弃段尾的 `.` 与空格, 先按创建结果对齐, 记录与磁盘才一致.
/// `.` 与 `..` 原样保留, 逃逸检查依赖它们.
fn clean_segment(part: &str) -> String {
    if part == "." || part == ".." {
        return part.to_owned();
    }
    let mut cleaned = String::with_capacity(part.len());
    for character in part.chars() {
        if character.is_control() {
            cleaned.push(' ');
        } else {
            cleaned.push(character);
        }
    }
    cleaned.trim_end_matches(['.', ' ']).to_owned()
}

/// 模板是否产出绝对路径: 模板源以 `/`、`\` 或盘符开头.
///
/// 锚必须看模板源而不是渲染结果: 相对模板 `{maker}/…` 在 maker 为空时渲染出前导 `/`,
/// 若据此保留锚, 空段折叠就会被破坏, 空目录段悄悄变成根目录.
fn anchor_of(source: &str) -> String {
    let trimmed = source.trim_start();
    if trimmed.starts_with('/') {
        return "/".to_owned();
    }
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return format!("{}:/", trimmed[..1].to_ascii_uppercase());
    }
    String::new()
}

/// 规范化渲染结果的每个路径段并丢弃空段.
///
/// 锚从模板源切出并原样保留; 锚之后的一切空段都丢弃: 空占位符不能留下空目录.
fn normalize_segments(source: &str, filled: &str) -> String {
    let anchor = anchor_of(source);
    let posix = filled.replace('\\', "/");
    let body = posix.strip_prefix(&anchor).unwrap_or(&posix);
    let parts: Vec<String> = body
        .split('/')
        .map(clean_segment)
        .filter(|part| !part.is_empty())
        .collect();
    let joined = parts.join("/");
    if parts.is_empty() {
        return anchor.trim_end_matches('/').to_owned();
    }
    format!("{anchor}{joined}")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Template, TemplateError, clip_bytes, sanitize_field};

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn renders_placeholders_and_folds_empty_groups() {
        let template =
            Template::parse("{maker}/{number}/{number}[-{title}].{ext}").expect("应解析成功");

        let filled = template.render(&vars(&[
            ("maker", "SOD"),
            ("number", "ABC-123"),
            ("title", "标题"),
            ("ext", "mp4"),
        ]));
        assert_eq!(filled, "SOD/ABC-123/ABC-123-标题.mp4");

        // title 为空: 所在可选组整段折叠, 不留下 `ABC-123-.mp4`.
        let folded = template.render(&vars(&[
            ("maker", "SOD"),
            ("number", "ABC-123"),
            ("ext", "mp4"),
        ]));
        assert_eq!(folded, "SOD/ABC-123/ABC-123.mp4");
    }

    #[test]
    fn empty_field_collapses_whole_segment() {
        // maker 为空: 目录段整段消失, 不会得到 `/ABC-123.mp4`.
        let template = Template::parse("{maker}/{number}.{ext}").expect("应解析成功");
        let rendered = template.render(&vars(&[("number", "ABC-123"), ("ext", "mp4")]));
        assert_eq!(rendered, "ABC-123.mp4");
    }

    #[test]
    fn nested_group_decides_independently() {
        // 同一组内两个直接占位符: 全空折叠, 有一个非空则整组渲染.
        let template = Template::parse("{number}[/{series}[-{label}]].{ext}").expect("应解析成功");

        let both = template.render(&vars(&[
            ("number", "ABC-123"),
            ("series", "系列"),
            ("label", "厂牌"),
            ("ext", "mp4"),
        ]));
        assert_eq!(both, "ABC-123/系列-厂牌.mp4");

        let outer_only = template.render(&vars(&[
            ("number", "ABC-123"),
            ("series", "系列"),
            ("ext", "mp4"),
        ]));
        assert_eq!(outer_only, "ABC-123/系列.mp4");

        let none = template.render(&vars(&[("number", "ABC-123"), ("ext", "mp4")]));
        assert_eq!(none, "ABC-123.mp4");

        // `[[...]]` wrap 组: 有值时包一层方括号.
        let template = Template::parse("{number}[[-{label}]].{ext}").expect("应解析成功");
        let wrapped = template.render(&vars(&[
            ("number", "ABC-123"),
            ("label", "厂牌"),
            ("ext", "mp4"),
        ]));
        assert_eq!(wrapped, "ABC-123[-厂牌].mp4");
        let unwrapped = template.render(&vars(&[("number", "ABC-123"), ("ext", "mp4")]));
        assert_eq!(unwrapped, "ABC-123.mp4");
    }

    #[test]
    fn mapping_rewrites_values() {
        let template =
            Template::parse("{content_type|uc=有码,fc2=FC2}/{number}.{ext}").expect("应解析成功");
        let rendered = template.render(&vars(&[
            ("content_type", "fc2"),
            ("number", "FC2-1234567"),
            ("ext", "mp4"),
        ]));
        assert_eq!(rendered, "FC2/FC2-1234567.mp4");

        // `{name|=缺省}`: 空源映成缺省.
        let template = Template::parse("{series|=未知}/{number}.{ext}").expect("应解析成功");
        let rendered = template.render(&vars(&[("number", "ABC-123"), ("ext", "mp4")]));
        assert_eq!(rendered, "未知/ABC-123.mp4");

        // 未列入映射的取值原样输出.
        let template =
            Template::parse("{content_type|uc=有码}/{number}.{ext}").expect("应解析成功");
        let rendered = template.render(&vars(&[
            ("content_type", "amateur"),
            ("number", "111111_001"),
            ("ext", "mp4"),
        ]));
        assert_eq!(rendered, "amateur/111111_001.mp4");
    }

    #[test]
    fn illegal_characters_are_stripped_per_windows_rules() {
        // Windows 保留字符剔除; 目录分隔符与 `:` 替换为空格.
        assert_eq!(sanitize_field("a<b>c:d\"e|f?g*h"), "abc defgh");
        assert_eq!(sanitize_field("a/b\\c:d"), "a b c d");
        assert_eq!(
            sanitize_field("bad\u{7}name"),
            "bad name",
            "控制字符替换为空格"
        );
        assert_eq!(sanitize_field("  "), "");

        // 渲染后的段按创建结果对齐: 段尾 `.` 与空格剥除.
        let template = Template::parse("{number}").expect("应解析成功");
        let rendered = template.render(&vars(&[("number", "ABC-123.")]));
        assert_eq!(rendered, "ABC-123");
    }

    #[test]
    fn overlong_segments_are_clipped_by_bytes() {
        // 100 个汉字 = 300 字节, 超出 200 上限.
        let long_title = "汉".repeat(100);
        let template = Template::parse("{number}[-{title}].{ext}").expect("应解析成功");
        let rendered = template.render(&vars(&[
            ("number", "ABC-123"),
            ("title", &long_title),
            ("ext", "mp4"),
        ]));

        let stem = rendered
            .trim_end_matches(".mp4")
            .trim_start_matches("ABC-123-");
        assert!(
            stem.len() <= super::MAX_SEGMENT_BYTES,
            "段不得超过 {} 字节, 实际 {}",
            super::MAX_SEGMENT_BYTES,
            stem.len()
        );
        assert!(stem.ends_with(super::ELLIPSIS), "应带省略号: {stem}");
        assert!(stem.starts_with("汉"), "多字节字符不得被切开");
    }

    #[test]
    fn clip_bytes_never_splits_multibyte_characters() {
        let value = "汉".repeat(80);
        let clipped = clip_bytes(&value, 200);
        // 197 字节预算不落在字符边界, 回退到 195 字节 = 65 个汉字; 省略号 3 字节.
        assert_eq!(clipped.chars().count(), 66, "65 汉字 + 省略号");
        assert_eq!(clipped.len(), 198);
    }

    #[test]
    fn absolute_template_keeps_its_anchor() {
        let template = Template::parse("/mnt/{number}.{ext}").expect("应解析成功");
        let rendered = template.render(&vars(&[("number", "ABC-123"), ("ext", "mp4")]));
        assert_eq!(rendered, "/mnt/ABC-123.mp4");

        // 中间的空段只影响该段, 不吞掉锚.
        let template = Template::parse("/mnt/{series}/{number}.{ext}").expect("应解析成功");
        let rendered = template.render(&vars(&[("number", "ABC-123"), ("ext", "mp4")]));
        assert_eq!(rendered, "/mnt/ABC-123.mp4");

        let template = Template::parse("D:\\media\\{number}.{ext}").expect("应解析成功");
        let rendered = template.render(&vars(&[("number", "ABC-123"), ("ext", "mp4")]));
        assert_eq!(rendered, "D:/media/ABC-123.mp4");
    }

    #[test]
    fn parser_rejects_malformed_templates() {
        let cases = [
            ("{number", TemplateError::UnclosedPlaceholder),
            ("{number}[title", TemplateError::UnclosedGroup),
            ("{number}]", TemplateError::StrayBracket),
            ("{}abc", TemplateError::EmptyPlaceholder),
            ("{maker{nested}}", TemplateError::NestedBrace),
            (
                "{plot}",
                TemplateError::UnknownPlaceholder("plot".to_owned()),
            ),
            (
                "{number|novalue}",
                TemplateError::BadMapping("number".to_owned()),
            ),
            ("{number|}", TemplateError::BadMapping("number".to_owned())),
            (
                "{number|a=x,a=y}",
                TemplateError::DuplicateMappingKey("a".to_owned()),
            ),
        ];
        for (source, expected) in cases {
            assert_eq!(
                Template::parse(source).expect_err(source),
                expected,
                "模板 {source} 应报对应错误"
            );
        }
    }

    #[test]
    fn repeated_renders_are_identical() {
        let template = Template::parse("{maker}/{number}[-{title}].{ext}").expect("应解析成功");
        let variables = vars(&[
            ("maker", "SOD"),
            ("number", "ABC-123"),
            ("title", "标题"),
            ("ext", "mp4"),
        ]);
        assert_eq!(template.render(&variables), template.render(&variables));
    }
}
