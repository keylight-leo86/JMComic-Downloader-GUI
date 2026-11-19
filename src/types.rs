//! 实体模型：本子（album）与章节（photo）。
//!
//! 对应 Python 版 `jm_entity.py` 中下载流程真正用到的部分。

use crate::config::DEFAULT_IMAGE_DOMAINS;

/// 本子中的一个章节条目（来自 album 的 `series` 字段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Episode {
    pub photo_id: String,
    /// 章节在本子中的序号，从 1 开始。
    pub index: u32,
    pub name: String,
}

/// 本子详情。
#[derive(Debug, Clone)]
pub struct Album {
    pub id: String,
    pub name: String,
    pub authors: Vec<String>,
    pub tags: Vec<String>,
    pub description: String,
    pub page_count: u32,
    pub scramble_id: String,
    pub episodes: Vec<Episode>,
}

impl Album {
    pub fn author(&self) -> &str {
        self.authors
            .first()
            .map(String::as_str)
            .unwrap_or("default_author")
    }

    /// 是否只有一章（此时 PDF 直接用本子名）。
    pub fn is_single_chapter(&self) -> bool {
        self.episodes.len() <= 1
    }
}

/// 章节详情（含图片列表时才能下载图片）。
#[derive(Debug, Clone)]
pub struct Photo {
    pub id: String,
    pub name: String,
    /// 章节所属本子 ID：单章本子时等于自身 ID。
    pub album_id: String,
    /// 章节在本子中的序号，从 1 开始。
    pub index: u32,
    /// 原始 sort 值（单章本子可能是 2）。
    pub sort: u32,
    /// `series_id`，为 0 表示单章本子。
    pub series_id: i64,
    pub tags: Vec<String>,
    /// 图片解密所需的 scramble_id（可能为空字符串）。
    pub scramble_id: String,
    /// 图片 CDN 域名。
    pub image_domain: String,
    /// 图片文件名列表，例如 `00001.webp`。
    pub images: Vec<String>,
    /// 所属本子详情（用于目录名与 PDF 命名）。
    pub album: Option<Album>,
}

impl Photo {
    pub fn is_single_album(&self) -> bool {
        self.series_id == 0
    }

    pub fn image_url(&self, index: usize) -> String {
        self.image_url_with_domain(index, &self.image_domain)
    }

    pub fn image_url_with_domain(&self, index: usize, domain: &str) -> String {
        let name = self
            .images
            .get(index)
            .map(String::as_str)
            .unwrap_or("00001.webp");
        crate::config::join_url(domain, &format!("/media/photos/{}/{name}", self.id))
    }

    /// 图片文件名去掉后缀，例如 `00001`。
    pub fn image_stem(&self, index: usize) -> &str {
        let name = self
            .images
            .get(index)
            .map(String::as_str)
            .unwrap_or("00001.webp");
        match name.rfind('.') {
            Some(pos) => &name[..pos],
            None => name,
        }
    }

    /// 图片后缀，例如 `.webp`。
    pub fn image_suffix(&self, index: usize) -> &str {
        let name = self
            .images
            .get(index)
            .map(String::as_str)
            .unwrap_or("00001.webp");
        match name.rfind('.') {
            Some(pos) => &name[pos..],
            None => "",
        }
    }

    pub fn is_gif(&self, index: usize) -> bool {
        self.image_suffix(index).eq_ignore_ascii_case(".gif")
    }
}

/// 生成一个随机图片 CDN 域名（简单轮询，避免依赖 rand）。
pub fn pick_image_domain(seed: usize) -> &'static str {
    DEFAULT_IMAGE_DOMAINS[seed % DEFAULT_IMAGE_DOMAINS.len()]
}

/// 判断文件名后缀是否为 jpg/jpeg（PDF 可直接嵌入的情况）。
pub fn suffix_is_jpeg(suffix: &str) -> bool {
    suffix.eq_ignore_ascii_case(".jpg") || suffix.eq_ignore_ascii_case(".jpeg")
}
