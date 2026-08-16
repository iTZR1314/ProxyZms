//! 节点备注:从 config.yaml 的 `proxies:` 段里把注释解析出来,给节点页当说明用。
//!
//! mihomo 的 `/proxies` 只回节点名和延迟,供应商 / 机房 / 是否住宅 IP 这些 API 里根本没有,
//! 但订阅作者本来就会在 YAML 里写注释 —— 直接读注释,换订阅备注就跟着换,不用改代码。
//!
//! 两种写法都认(同一节点都写了以行尾的为准):
//!
//! ```yaml
//! proxies:
//!   # 甲骨文 · 日本大阪 · IPv6          ← 紧贴节点上一行的整行注释
//!   - name: "Apollo"
//!   - name: "Venus"                     # 亚马逊 · 日本东京 · IPv4 / IPv6   ← 行尾注释
//! ```
//!
//! 用 `·`、`|`(或连续两个以上空格)分段,界面上就分列对齐;不分段则整条显示。
//! 某一段没有就留空:`# · 日本东京 · IPv4` —— 供应商列空着,地区仍然落在地区列。
//! 中间隔了空行的注释算「段落说明」,不会挂到下面的节点上。
//!
//! 注意:注释要写进**订阅源**里 —— 「更新订阅」会用下载到的内容整体覆盖 config.yaml。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// 一条节点备注:按分隔符切好的 1~3 段。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeNote {
    cols: Vec<String>,
}

impl NodeNote {
    /// 第 i 段;越界返回空串(界面上就是空白,不破坏列对齐)。
    pub fn col(&self, i: usize) -> &str {
        self.cols.get(i).map(String::as_str).unwrap_or("")
    }

    /// 分了几段。>= 2 时节点页按列渲染,否则整条塞一列。
    pub fn parts(&self) -> usize {
        self.cols.len()
    }

    /// 拼回完整文本(跳过占位的空列),用于悬停提示和单列渲染。
    pub fn text(&self) -> String {
        self.cols
            .iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// 节点名 → 备注。
pub type NoteMap = HashMap<String, NodeNote>;

/// config.yaml 的位置:`work_dir` 留空就是 bootstrap 托管的那份。
fn config_file(work_dir: &str) -> PathBuf {
    let dir = work_dir.trim();
    if dir.is_empty() {
        crate::bootstrap::config_path()
    } else {
        PathBuf::from(dir).join("config.yaml")
    }
}

struct Cached {
    path: PathBuf,
    /// (mtime, 文件大小);和上次一致就不重新解析
    stamp: Option<(SystemTime, u64)>,
    notes: Arc<NoteMap>,
}

static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

/// 取当前 config.yaml 里的节点备注。文件没动过就直接给缓存,每次只 stat 一下,
/// 所以可以在渲染里直接调(节点页 2 秒重绘一次)。读不到文件就是空表,不报错。
pub fn load(work_dir: &str) -> Arc<NoteMap> {
    let path = config_file(work_dir);
    let stamp = std::fs::metadata(&path)
        .ok()
        .and_then(|m| Some((m.modified().ok()?, m.len())));

    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = guard.as_ref() {
        if c.path == path && c.stamp == stamp {
            return c.notes.clone();
        }
    }
    let notes = Arc::new(
        std::fs::read_to_string(&path)
            .map(|s| parse(&s))
            .unwrap_or_default(),
    );
    *guard = Some(Cached {
        path,
        stamp,
        notes: notes.clone(),
    });
    notes
}

/// 逐行扫 `proxies:` 段,把注释挂到节点名上。
/// 不用 YAML 解析器 —— 注释在语法树里根本不存在,只能按行读原文。
pub fn parse(yaml: &str) -> NoteMap {
    let mut out = NoteMap::new();
    let mut in_proxies = false;
    // 紧贴在条目上一行的整行注释(空行或代码行都会把它清掉)
    let mut pending: Option<String> = None;
    // 当前条目已取到的备注 / 是否还在等它的 name
    let mut entry_note: Option<String> = None;
    let mut awaiting_name = false;

    for line in yaml.lines() {
        // 顶层键换段:proxies 之外的内容一律不看
        if !line.is_empty() && !line.starts_with([' ', '\t']) {
            in_proxies = line.split(':').next().unwrap_or("").trim() == "proxies";
            pending = None;
            entry_note = None;
            awaiting_name = false;
            continue;
        }
        if !in_proxies {
            continue;
        }

        let (code, comment) = split_comment(line);
        let code = code.trim();

        if code.is_empty() {
            // 纯注释行:记下来等下一个条目;空行:和下面的条目脱钩,
            // 这样「# 以下 socks5 落地经 Venus 中转」这类段落说明不会被当成节点备注
            pending = comment;
            if pending.is_none() {
                awaiting_name = false;
                entry_note = None;
            }
            continue;
        }

        if let Some(rest) = code.strip_prefix('-') {
            // 新条目开始:行尾注释优先,其次是紧贴上一行的注释
            entry_note = comment.clone().or_else(|| pending.take());
            awaiting_name = true;
            if let Some(name) = parse_name(rest) {
                record(&mut out, name, entry_note.take());
                awaiting_name = false;
            }
        } else if awaiting_name {
            // 条目里 name 单独占一行的写法
            if let Some(name) = parse_name(code) {
                record(
                    &mut out,
                    name,
                    comment.clone().or_else(|| entry_note.take()),
                );
                awaiting_name = false;
            }
        }
        pending = None;
    }
    out
}

fn record(out: &mut NoteMap, name: String, note: Option<String>) {
    let Some(note) = note else { return };
    let cols = split_cols(&note);
    if !cols.is_empty() {
        out.insert(name, NodeNote { cols });
    }
}

/// 切掉行尾注释,返回 (代码部分, 注释正文)。
/// `#` 要在行首或前面有空白才算注释(否则 `pass#word` 这种值会被腰斩),引号内的一律不算。
fn split_comment(line: &str) -> (&str, Option<String>) {
    let mut single = false;
    let mut double = false;
    let mut prev_ws = true;
    for (i, ch) in line.char_indices() {
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '#' if !single && !double && prev_ws => {
                let body = line[i..].trim_start_matches('#').trim();
                let body = if body.is_empty() {
                    None
                } else {
                    Some(body.to_string())
                };
                return (&line[..i], body);
            }
            _ => {}
        }
        prev_ws = ch.is_whitespace();
    }
    (line, None)
}

/// 从一段 YAML 代码里取 `name:` 的值,块写法(`name: "X"`)和流写法(`{ name: X, ... }`)都认。
fn parse_name(code: &str) -> Option<String> {
    let mut from = 0;
    while let Some(pos) = code[from..].find("name:") {
        let at = from + pos;
        // 前一个字符必须是分隔符,免得 `username:` 也被当成 name
        let ok = code[..at]
            .chars()
            .last()
            .map(|c| c == '{' || c == ',' || c.is_whitespace())
            .unwrap_or(true);
        if ok {
            return Some(take_value(&code[at + "name:".len()..]));
        }
        from = at + "name:".len();
    }
    None
}

/// 取到 `,` 或 `}` 为止的值,顺手脱掉引号。
fn take_value(rest: &str) -> String {
    let rest = rest.trim_start();
    if let Some(inner) = rest.strip_prefix(['"', '\'']) {
        let quote = rest.chars().next().unwrap_or('"');
        return inner.split(quote).next().unwrap_or("").to_string();
    }
    rest.split([',', '}'])
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

/// 把注释切成最多 3 段:优先按 `·`/`•`/`|` 这类分隔符,没有就按「两个以上空格」切。
/// 空段会保留成占位空列 —— 没有供应商的节点写 `# · 日本东京 · IPv4`,地区就仍落在地区列。
/// 超过 3 段的把尾巴并进第 3 段,保证列数固定、界面能对齐。
fn split_cols(note: &str) -> Vec<String> {
    const SEPS: [char; 4] = ['·', '•', '|', '｜'];
    let mut cols: Vec<String> = note.split(SEPS).map(|s| s.trim().to_string()).collect();
    // 末尾的空段只是手滑多打的分隔符,占位没有意义
    while cols.last().is_some_and(|s| s.is_empty()) {
        cols.pop();
    }
    if cols.len() < 2 {
        cols = split_wide_gaps(note);
    }
    if cols.len() > 3 {
        let tail: Vec<String> = cols
            .split_off(2)
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
        cols.push(tail.join(" · "));
    }
    cols
}

/// 按「连续两个以上空格 / 全角空格」分段 —— 手写注释里最自然的对齐方式。
fn split_wide_gaps(note: &str) -> Vec<String> {
    let flat = note.replace(['\u{3000}', '\t'], "  ");
    let mut cols = Vec::new();
    let mut cur = String::new();
    let mut gap = 0usize;
    for ch in flat.chars() {
        if ch == ' ' {
            gap += 1;
            continue;
        }
        if gap >= 2 && !cur.trim().is_empty() {
            cols.push(cur.trim().to_string());
            cur.clear();
        } else if gap > 0 && !cur.is_empty() {
            cur.push(' ');
        }
        gap = 0;
        cur.push(ch);
    }
    if !cur.trim().is_empty() {
        cols.push(cur.trim().to_string());
    }
    cols
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
mixed-port: 7890
proxies:
  - name: "Apollo"    # 甲骨文 · 日本大阪 · IPv6
    type: hysteria2
    password: "aa#bb"
    server: apollo.example.com

  # 亚马逊 · 日本东京 · IPv4 / IPv6
  - name: "Venus"
    type: hysteria2

  # 以下 socks5 落地经 Venus 中转

  - name: "US-PaloAlto"   # 住宅 IP   美国帕洛阿托
    type: socks5

  - { name: JP-Tokyo, type: socks5 }   # 住宅 IP | 日本东京
  - name: "NoNote"
    type: socks5
  - name: "NoVendor"   # · 日本东京 · IPv4
    type: socks5
  -
    name: "Blocky"    # 供应商 · 地区 · IPv4 · 备用 · 多余
proxy-groups:
  - { name: 阿波罗, type: select }   # 这条不该被解析
rules:
  - 'DOMAIN,x,DIRECT'
"#;

    #[test]
    fn parses_notes() {
        let m = parse(YAML);
        assert_eq!(m["Apollo"].cols, ["甲骨文", "日本大阪", "IPv6"]);
        assert_eq!(m["Venus"].cols, ["亚马逊", "日本东京", "IPv4 / IPv6"]);
        // 行尾注释按「两个以上空格」分段
        assert_eq!(m["US-PaloAlto"].cols, ["住宅 IP", "美国帕洛阿托"]);
        // 流写法 + | 分隔
        assert_eq!(m["JP-Tokyo"].cols, ["住宅 IP", "日本东京"]);
        // name 单独一行
        assert_eq!(m["Blocky"].cols, ["供应商", "地区", "IPv4 · 备用 · 多余"]);
        // 前导分隔符 = 空占位列,后面的段不会左移
        assert_eq!(m["NoVendor"].cols, ["", "日本东京", "IPv4"]);
        assert_eq!(m["NoVendor"].text(), "日本东京 · IPv4");
        assert!(!m.contains_key("NoNote"));
        // 空行隔开的段落说明不挂到节点上;proxy-groups 不解析
        assert!(!m.values().any(|n| n.text().contains("中转")));
        assert!(!m.contains_key("阿波罗"));
    }

    #[test]
    fn quoted_hash_is_not_a_comment() {
        let (code, comment) = split_comment(r#"    password: "aa#bb""#);
        assert!(code.contains("aa#bb"));
        assert_eq!(comment, None);
    }
}

