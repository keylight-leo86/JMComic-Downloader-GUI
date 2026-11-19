//! 网页端（HTML）解析层：把本子页 / 章节页解析成实体。
//!
//! 对照 Python 版 `jm_toolkit.py` 中的 `JmcomicText`：
//! - `pattern_html_album_*` / `pattern_html_photo_*` 这组正则；
//! - `parse_jm_base64_html`（网页端会把 html 藏在 base64 里）；
//! - `analyse_jm_album_html` / `analyse_jm_photo_html` 的取值与默认值语义；
//! - `JmAlbumDetail.distinct_episode`（按序号升序 + 同序号去重）。
//!
//! 所有正则都由 `OnceLock` 只编译一次，避免每次解析都重新编译。

use std::sync::OnceLock;

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use base64::Engine as _;
use regex::Regex;

use crate::error::{JmError, Result};
use crate::types::{pick_image_domain, Album, Episode, Photo};

/// 编译静态正则：失败只可能是字面量写错，属于编码错误，直接 panic 暴露。
macro_rules! static_regex {
    ($text:expr) => {
        Regex::new($text).expect(concat!("静态正则编译失败: ", $text))
    };
}

// ---------------------------------------------------------------------------
// 正则集合
// ---------------------------------------------------------------------------

/// 域名相关正则（发布页域名候选、从 URL 取域名）。
struct DomainPatterns {
    /// 发布页里出现的域名候选（Python: `pattern_html_jm_pub_domain`）
    pub_domain: Regex,
    /// 从 URL 中取域名（Python: `pattern_jm_domain`）
    url_domain: Regex,
}

impl DomainPatterns {
    fn new() -> Self {
        Self {
            pub_domain: static_regex!(r"[\w-]+\.\w+/?\w+"),
            url_domain: static_regex!(r"https://([\w.-]+)"),
        }
    }
}

fn domain_patterns() -> &'static DomainPatterns {
    static PATTERNS: OnceLock<DomainPatterns> = OnceLock::new();
    PATTERNS.get_or_init(DomainPatterns::new)
}

/// 本子页正则集合（Python: `pattern_html_album_*`）。
struct AlbumPatterns {
    album_id: Regex,
    scramble_id: Regex,
    name: Regex,
    description: Regex,
    episode_list: Regex,
    page_count: Regex,
    likes: Regex,
    views: Regex,
    comment_count: Regex,
    author_outer: Regex,
    works_outer: Regex,
    actor_outer: Regex,
    tags_outer: Regex,
    tag_a: Regex,
}

impl AlbumPatterns {
    fn new() -> Self {
        Self {
            album_id: static_regex!(r#"<span class="number">.*?：JM(\d+)</span>"#),
            scramble_id: static_regex!(r"var scramble_id = (\d+);"),
            name: static_regex!(r#"id="book-name"[^>]*?>([\s\S]*?)<"#),
            description: static_regex!(r"[叙|敘]述：([\s\S]*?)</h2>"),
            episode_list: static_regex!(
                r#"data-album="(\d+)"[^>]*>[\s\S]*?第(\d+)[话話]([\s\S]*?)<[\s\S]*?>"#
            ),
            page_count: static_regex!(r#"<span class="pagecount">.*?:(\d+)</span>"#),
            likes: static_regex!(r#"<span id="albim_likes_\d+">(.*?)</span>"#),
            views: static_regex!(
                r#"<span>(.*?)</span>\n *<span>(次觀看|观看次数|次观看次数)</span>"#
            ),
            comment_count: static_regex!(
                r#"<div class="badge"[^>]*?id="total_video_comments">(\d+)</div>"#
            ),
            author_outer: static_regex!(
                r#"<span itemprop="author" data-type="author">([\s\S]*?)</span>"#
            ),
            works_outer: static_regex!(
                r#"<span itemprop="author" data-type="works">([\s\S]*?)</span>"#
            ),
            actor_outer: static_regex!(
                r#"<span itemprop="author" data-type="actor">([\s\S]*?)</span>"#
            ),
            tags_outer: static_regex!(
                r#"<span itemprop="genre" data-type="tags">([\s\S]*?)</span>"#
            ),
            tag_a: static_regex!(r"<a[^>]*?>\s*(\S*)\s*</a>"),
        }
    }
}

fn album_patterns() -> &'static AlbumPatterns {
    static PATTERNS: OnceLock<AlbumPatterns> = OnceLock::new();
    PATTERNS.get_or_init(AlbumPatterns::new)
}

/// 章节页正则集合（Python: `pattern_html_photo_*`）。
struct PhotoPatterns {
    photo_id: Regex,
    scramble_id: Regex,
    name: Regex,
    image_domain: Regex,
    series_id: Regex,
    sort: Regex,
    page_arr: Regex,
    tags: Regex,
}

impl PhotoPatterns {
    fn new() -> Self {
        Self {
            photo_id: static_regex!(r#"<meta property="og:url" content=".*?/photo/(\d+)/?.*?">"#),
            scramble_id: static_regex!(r"var scramble_id = (\d+);"),
            name: static_regex!(r"<title>([\s\S]*?)\|.*</title>"),
            image_domain: static_regex!(r#"src="https://(.*?)/media/albums/blank"#),
            series_id: static_regex!(r"var series_id = (\d+);"),
            sort: static_regex!(r"var sort = (\d+);"),
            page_arr: static_regex!(r"var page_arr = (.*?);"),
            tags: static_regex!(r#"<meta name="keywords"[\s\S]*?content="(.*?)""#),
        }
    }
}

fn photo_patterns() -> &'static PhotoPatterns {
    static PATTERNS: OnceLock<PhotoPatterns> = OnceLock::new();
    PATTERNS.get_or_init(PhotoPatterns::new)
}

/// base64 包裹正则（Python: `pattern_html_b64_decode_content`，`.` 不匹配换行）。
fn b64_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| static_regex!(r#"const html = base64DecodeUtf8\("(.*?)"\)"#))
}

// ---------------------------------------------------------------------------
// 通用小工具
// ---------------------------------------------------------------------------

/// 取第一个捕获组；未匹配返回 `None`（对照 Python 的 `match[1]`）。
fn match_group1(pattern: &Regex, text: &str) -> Option<String> {
    pattern.captures(text).map(|caps| caps[1].to_string())
}

/// 两段式提取：先用外层 span 缩小范围，再用 `<a>` 正则取链接文本。
/// 外层没匹配上时返回空 Vec（不报错，对照 Python 的 pattern list 语义）。
fn collect_tagged(outer: &Regex, tag_a: &Regex, text: &str) -> Vec<String> {
    let inner = match outer.captures(text) {
        Some(caps) => caps[1].to_string(),
        None => return Vec::new(),
    };
    tag_a
        .captures_iter(&inner)
        .map(|caps| caps[1].to_string())
        .collect()
}

/// 截断文本，避免错误信息里塞进整页 html。
fn brief_text(text: &str) -> String {
    const LIMIT: usize = 80;
    let mut out: String = text.chars().take(LIMIT).collect();
    if text.chars().count() > LIMIT {
        out.push_str("...");
    }
    out
}

/// 宽松的 base64 解码：先按标准字母表，再兼容缺失 padding 的情况。
fn decode_b64_lenient(raw: &str) -> Option<Vec<u8>> {
    if let Ok(bytes) = STANDARD.decode(raw) {
        return Some(bytes);
    }
    let unpadded = raw.trim_end_matches('=');
    STANDARD_NO_PAD.decode(unpadded).ok()
}

// ---------------------------------------------------------------------------
// 公开 API
// ---------------------------------------------------------------------------

/// 解开网页端的 base64 包裹：`const html = base64DecodeUtf8("...")`。
///
/// 未匹配包裹、解码失败或结果不是合法 UTF-8 时，原样返回入参。
pub fn decode_base64_wrapped(text: &str) -> String {
    let captured = match b64_pattern().captures(text) {
        Some(caps) => caps[1].trim().to_string(),
        None => return text.to_string(),
    };
    if captured.is_empty() {
        return text.to_string();
    }

    match decode_b64_lenient(&captured) {
        Some(bytes) => match String::from_utf8(bytes) {
            Ok(html) => html,
            Err(_) => text.to_string(),
        },
        None => text.to_string(),
    }
}

/// 从发布页 html 中提取候选域名（含 "jm" 或 "comic"），按首次出现顺序去重。
pub fn extract_domain_list(html: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for found in domain_patterns().pub_domain.find_iter(html) {
        let domain = found.as_str();
        // 对照 Python: any(kw in domain for kw in ('jm', 'comic'))
        if !(domain.contains("jm") || domain.contains("comic")) {
            continue;
        }
        if out.iter().any(|kept| kept == domain) {
            continue;
        }
        out.push(domain.to_string());
    }
    out
}

/// 从 URL 中取域名，例如 `https://jm-comic2.cc/xxx` → `jm-comic2.cc`。
pub fn host_from_url(url: &str) -> Option<String> {
    let host = match domain_patterns().url_domain.captures(url) {
        Some(caps) => caps[1].to_string(),
        None => return None,
    };
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// 解析本子页面。
///
/// 必填字段：本子 id、本子名；其余字段缺失时使用与 Python 版一致的默认值。
pub fn parse_album_html(html: &str) -> Result<Album> {
    // 网页端本子页的 html 常被 base64 包裹一层
    let text = decode_base64_wrapped(html);
    let p = album_patterns();

    let id = match match_group1(&p.album_id, &text) {
        Some(id) if !id.is_empty() => id,
        _ => {
            return JmError::msg(format!(
                "网页解析失败：未匹配到本子 id（pattern_html_album_album_id）；响应文本={}",
                brief_text(&text)
            ))
        }
    };

    let name = match p.name.captures(&text) {
        Some(caps) if !caps[1].trim().is_empty() => caps[1].trim().to_string(),
        _ => return JmError::msg(format!("网页解析失败：未匹配到本子名（album-{id}）")),
    };

    let scramble_id = match_group1(&p.scramble_id, &text).unwrap_or_else(|| "0".to_string());

    let description = match_group1(&p.description, &text)
        .map(|text| text.trim().to_string())
        .unwrap_or_default();

    let page_count = match_group1(&p.page_count, &text)
        .and_then(|text| text.trim().parse::<u32>().ok())
        .unwrap_or(0);

    let authors = collect_tagged(&p.author_outer, &p.tag_a, &text);
    let tags = collect_tagged(&p.tags_outer, &p.tag_a, &text);

    // 目标 `Album` 结构体没有 works / actors / likes / views / comment_count 字段，
    // 这里仍按 Python 版解析一遍，保持字段覆盖一致（结果暂不使用）。
    let _works = collect_tagged(&p.works_outer, &p.tag_a, &text);
    let _actors = collect_tagged(&p.actor_outer, &p.tag_a, &text);
    let _likes = match_group1(&p.likes, &text).unwrap_or_default();
    let _views = match_group1(&p.views, &text).unwrap_or_default();
    let _comment_count = match_group1(&p.comment_count, &text).unwrap_or_else(|| "0".to_string());

    let mut episodes = parse_episodes(&p.episode_list, &text);
    if episodes.is_empty() {
        // 没有章节列表时自成一章（对照 JmAlbumDetail.__init__）
        episodes.push(Episode {
            photo_id: id.clone(),
            index: 1,
            name: name.clone(),
        });
    }

    Ok(Album {
        id,
        name,
        authors,
        tags,
        description,
        page_count,
        scramble_id,
        episodes,
    })
}

/// 解析章节页面（含图片列表原始文件名）。
pub fn parse_photo_html(html: &str) -> Result<Photo> {
    let p = photo_patterns();

    let photo_id = match match_group1(&p.photo_id, html) {
        Some(id) if !id.is_empty() => id,
        _ => {
            return JmError::msg(format!(
                "网页解析失败：未匹配到章节 id（pattern_html_photo_photo_id）；响应文本={}",
                brief_text(html)
            ))
        }
    };

    let scramble_id = match_group1(&p.scramble_id, html).unwrap_or_else(|| "0".to_string());

    // 标题缺失时用 photo-{id} 兜底，避免整章失败
    let name = match p.name.captures(html) {
        Some(caps) if !caps[1].trim().is_empty() => caps[1].trim().to_string(),
        _ => format!("photo-{photo_id}"),
    };

    let photo_num = photo_id.parse::<usize>().unwrap_or(0);

    let image_domain = match match_group1(&p.image_domain, html) {
        Some(domain) if !domain.trim().is_empty() => domain.trim().to_string(),
        // 页面上没有图片域名时，按 id 轮询内置 CDN 域名
        _ => pick_image_domain(photo_num).to_string(),
    };

    let series_id = match_group1(&p.series_id, html)
        .and_then(|text| text.trim().parse::<i64>().ok())
        .unwrap_or(0);

    // 单章本子 JM 给的 sort 是 2
    let sort = match_group1(&p.sort, html)
        .and_then(|text| text.trim().parse::<u32>().ok())
        .unwrap_or(1);

    let raw_page_arr = match match_group1(&p.page_arr, html) {
        Some(text) => text,
        None => {
            return JmError::msg(format!(
                "无法解析图片列表：未匹配到 page_arr（photo-{photo_id}）"
            ))
        }
    };
    let images: Vec<String> = match serde_json::from_str::<Vec<String>>(raw_page_arr.trim()) {
        Ok(list) => list,
        Err(err) => {
            return JmError::msg(format!(
                "无法解析图片列表（page_arr）：{err}；原始文本={}",
                brief_text(&raw_page_arr)
            ))
        }
    };

    let tags: Vec<String> = match match_group1(&p.tags, html) {
        Some(text) => text
            .split(',')
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .map(str::to_string)
            .collect(),
        None => Vec::new(),
    };

    let index = if series_id == 0 && sort == 2 { 1 } else { sort };
    let album_id = if series_id == 0 {
        photo_id.clone()
    } else {
        series_id.to_string()
    };

    Ok(Photo {
        id: photo_id,
        name,
        album_id,
        index,
        sort,
        series_id,
        tags,
        scramble_id,
        image_domain,
        images,
        album: None,
    })
}

/// 章节列表：按序号升序（稳定排序）后同序号去重，保留首次出现的条目。
/// 对照 Python `JmAlbumDetail.distinct_episode`。
fn parse_episodes(pattern: &Regex, text: &str) -> Vec<Episode> {
    let mut episodes: Vec<Episode> = pattern
        .captures_iter(text)
        .map(|caps| Episode {
            photo_id: caps[1].to_string(),
            // 序号解析失败时按“从 1 开始”的语义兜底
            index: caps[2].parse::<u32>().unwrap_or(1),
            name: caps[3].trim().to_string(),
        })
        .collect();

    episodes.sort_by_key(|episode| episode.index);

    let mut seen: Vec<u32> = Vec::with_capacity(episodes.len());
    episodes.retain(|episode| {
        if seen.contains(&episode.index) {
            false
        } else {
            seen.push(episode.index);
            true
        }
    });

    episodes
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 本子页：含 2 个有效序号 + 1 个重复序号（第 1 話出现了两次）。
    const ALBUM_HTML: &str = r#"<html><head><title>t</title></head><body>
<span class="number">：JM123456</span>
<h1 id="book-name">测试本子</h1>
<h2>敘述：这是描述</h2>
<a data-album="111" href="/photo/111">第2話 第二章</a>
<a data-album="222" href="/photo/222">第1話 第一章</a>
<a data-album="333" href="/photo/333">第1話 重复章节</a>
<span class="pagecount">總頁數:12</span>
<span itemprop="author" data-type="author"><a href="/search?author=a">作者甲</a></span>
<span itemprop="genre" data-type="tags"><a href="/tag/1">全彩</a><a href="/tag/2">中文</a></span>
<script>var scramble_id = 220980;</script>
</body></html>"#;

    /// 章节页：多章本子（series_id != 0）。
    const PHOTO_HTML: &str = r#"<html><head>
<meta property="og:url" content="https://18comic.vip/photo/123456/">
<title>第42話 癡漢 | 禁漫天堂</title>
<meta name="keywords" content="標籤A, 標籤B ,">
</head><body>
<img src="https://cdn-msp2.jmapiproxy2.cc/media/albums/blank.jpg">
<script>
var scramble_id = 220980;
var series_id = 400222;
var sort = 42;
var page_arr = ["00001.webp","00002.webp"];
</script>
</body></html>"#;

    /// 章节页：单章本子（series_id == 0 且 sort == 2）。
    const PHOTO_SINGLE_HTML: &str = r#"<html><head>
<meta property="og:url" content="https://18comic.vip/photo/777888/">
<title>单章标题 | 禁漫天堂</title>
</head><body>
<script>
var series_id = 0;
var sort = 2;
var page_arr = ["00001.jpg"];
</script>
</body></html>"#;

    /// 把本子页包成网页端常见的 base64 形式。
    fn album_b64_page() -> String {
        let b64 = STANDARD.encode(ALBUM_HTML.as_bytes());
        format!(r#"<script>const html = base64DecodeUtf8("{b64}")</script>"#)
    }

    #[test]
    fn decodes_base64_wrapped_page() {
        let page = album_b64_page();
        assert_eq!(decode_base64_wrapped(&page), ALBUM_HTML);

        // 没有包裹时原样返回
        assert_eq!(decode_base64_wrapped(ALBUM_HTML), ALBUM_HTML);

        // 非法 base64 时原样返回
        let broken = r#"const html = base64DecodeUtf8("!!!!")"#;
        assert_eq!(decode_base64_wrapped(broken), broken);
    }

    #[test]
    fn parses_album_html() {
        let album = parse_album_html(ALBUM_HTML).expect("本子页应可解析");

        assert_eq!(album.id, "123456");
        assert_eq!(album.name, "测试本子");
        assert_eq!(album.description, "这是描述");
        assert_eq!(album.scramble_id, "220980");
        assert_eq!(album.page_count, 12);
        assert_eq!(album.authors, vec!["作者甲".to_string()]);
        assert_eq!(album.tags, vec!["全彩".to_string(), "中文".to_string()]);

        // 按序号升序，且序号 1 只保留文档中先出现的那一条
        assert_eq!(album.episodes.len(), 2);
        assert_eq!(album.episodes[0].photo_id, "222");
        assert_eq!(album.episodes[0].index, 1);
        assert_eq!(album.episodes[0].name, "第一章");
        assert_eq!(album.episodes[1].photo_id, "111");
        assert_eq!(album.episodes[1].index, 2);
        assert_eq!(album.episodes[1].name, "第二章");
    }

    #[test]
    fn parses_album_from_base64_page() {
        let album = parse_album_html(&album_b64_page()).expect("base64 包裹的本子页应可解析");
        assert_eq!(album.id, "123456");
        assert_eq!(album.name, "测试本子");
        assert_eq!(album.episodes.len(), 2);
    }

    #[test]
    fn album_without_episodes_becomes_single_episode() {
        let html = r#"<span class="number">：JM555</span><h1 id="book-name">单章本子</h1>"#;
        let album = parse_album_html(html).expect("本子页应可解析");

        assert_eq!(album.id, "555");
        assert_eq!(album.name, "单章本子");
        assert_eq!(album.description, "");
        assert_eq!(album.scramble_id, "0");
        assert_eq!(album.page_count, 0);
        assert!(album.authors.is_empty());
        assert!(album.tags.is_empty());
        assert_eq!(album.episodes.len(), 1);
        assert_eq!(album.episodes[0].photo_id, "555");
        assert_eq!(album.episodes[0].index, 1);
        assert_eq!(album.episodes[0].name, "单章本子");
    }

    #[test]
    fn parses_photo_html() {
        let photo = parse_photo_html(PHOTO_HTML).expect("章节页应可解析");

        assert_eq!(photo.id, "123456");
        assert_eq!(photo.name, "第42話 癡漢");
        assert_eq!(photo.series_id, 400222);
        assert_eq!(photo.album_id, "400222");
        assert_eq!(photo.sort, 42);
        assert_eq!(photo.index, 42);
        assert_eq!(photo.scramble_id, "220980");
        assert_eq!(photo.image_domain, "cdn-msp2.jmapiproxy2.cc");
        assert_eq!(
            photo.images,
            vec!["00001.webp".to_string(), "00002.webp".to_string()]
        );
        assert_eq!(photo.tags, vec!["標籤A".to_string(), "標籤B".to_string()]);
        assert!(photo.album.is_none());
        assert!(!photo.is_single_album());
    }

    #[test]
    fn parses_single_chapter_photo_html() {
        let photo = parse_photo_html(PHOTO_SINGLE_HTML).expect("单章页应可解析");

        assert_eq!(photo.id, "777888");
        assert_eq!(photo.name, "单章标题");
        assert_eq!(photo.series_id, 0);
        // 单章本子：album_id 回落到自身，sort=2 语义为第 1 话
        assert_eq!(photo.album_id, "777888");
        assert_eq!(photo.sort, 2);
        assert_eq!(photo.index, 1);
        // 页面里没有图片域名时使用内置 CDN 域名
        assert_eq!(photo.image_domain, pick_image_domain(777_888));
        assert!(!photo.image_domain.is_empty());
        // 没有 scramble_id / keywords 时使用默认值
        assert_eq!(photo.scramble_id, "0");
        assert!(photo.tags.is_empty());
        assert_eq!(photo.images, vec!["00001.jpg".to_string()]);
    }

    #[test]
    fn errors_when_required_fields_missing() {
        // 本子 id 缺失
        let err = parse_album_html("<html><body>没有车号</body></html>").unwrap_err();
        assert!(matches!(&err, JmError::Msg(_)));
        assert!(err.to_string().contains("本子 id"));

        // 本子名缺失
        let err = parse_album_html(r#"<span class="number">：JM999</span>"#).unwrap_err();
        assert!(err.to_string().contains("本子名"));

        // 章节 id 缺失
        assert!(parse_photo_html("<html></html>").is_err());

        // page_arr 缺失
        let html =
            r#"<meta property="og:url" content="https://x.cc/photo/1/"><title>n | s</title>"#;
        let err = parse_photo_html(html).unwrap_err();
        assert!(err.to_string().contains("图片列表"));

        // page_arr 不是合法的 JSON 字符串数组
        let html = r#"<meta property="og:url" content="https://x.cc/photo/1/">
<meta name="keywords" content="a,b">
<title>n | s</title>
var page_arr = ["00001.webp", 2];"#;
        let err = parse_photo_html(html).unwrap_err();
        assert!(err.to_string().contains("图片列表"));
    }

    #[test]
    fn extracts_domains_and_hosts() {
        let html = concat!(
            "见 https://jm-comic2.cc/abc 与 https://18comic.vip/def ",
            "以及 https://example.net/ghi https://jm-comic2.cc/abc"
        );
        assert_eq!(
            extract_domain_list(html),
            vec![
                "jm-comic2.cc/abc".to_string(),
                "18comic.vip/def".to_string()
            ]
        );

        assert_eq!(
            host_from_url("https://jm-comic2.cc/xxx").as_deref(),
            Some("jm-comic2.cc")
        );
        assert_eq!(host_from_url("http://jm-comic2.cc"), None);
        assert_eq!(host_from_url("不是链接"), None);
    }
}
