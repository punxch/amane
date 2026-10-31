//! FC2 官方电子市场. 地址由数字商品 ID 直接拼出, 不需要检索.
//!
//! 与 fc2ppvdb 的分工: 这条是官方站, 有官方简介与样品图; fc2ppvdb 是第三方库, 有演员与题材.

use std::sync::Arc;

use amane_core::MediaNumber;
use amane_net::{NetError, RequestOptions, WebClient};
use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::dom::{absolute, attrs, find_date, first_text, texts};
use crate::{Capability, RawBody, ScrapedMetadata, Source, SourceDescriptor, SourceError};

/// 请求实际打到的主机名; 限速按它分桶.
pub const HOST: &str = "adult.contents.fc2.com";

const BASE_URL: &str = "https://adult.contents.fc2.com";

const ID: &str = "fc2";

/// 商品页独有的块. 商品不在售时站点回一个没有该块的「找不到商品」页 (HTTP 200).
const ARTICLE_BLOCK: &str = r#"div[data-section="userInfo"]"#;

/// 不在售页面的标题标志. 站点按语言给两种标题, 英文与日文都要认 ——
/// 浏览器指纹默认带的是英文语言头, 只认日文会把下架判成改版.
const MISSING_MARKERS: &[&str] = &["見つかりません", "find any products"];

pub struct Fc2Source {
    client: Arc<WebClient>,
}

impl Fc2Source {
    pub fn new(client: Arc<WebClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Source for Fc2Source {
    fn descriptor(&self) -> &SourceDescriptor {
        &DESCRIPTOR
    }

    async fn fetch(&self, number: &MediaNumber) -> Result<ScrapedMetadata, SourceError> {
        let Some(article_id) = article_id(number) else {
            return Err(SourceError::NotFound {
                id: ID,
                number: number.as_str().to_owned(),
            });
        };

        // 地址由商品 ID 拼出, 因此一次商品页请求同时充当「在售确认」与「取数」.
        let url = format!("{BASE_URL}/article/{article_id}/");
        let html = self
            .client
            .get_text_with(&url, &options())
            .await
            .map_err(transport_error)?;

        parse_detail(&html, &article_id)
    }
}

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: ID,
    label: "FC2",
    base_url: BASE_URL,
    hosts: &[HOST],
    rate: amane_net::Rate::each_second(1, 1),
    capabilities: &[Capability::Metadata],
};

/// 站点要求的请求头.
///
/// 语言头决定标签文字与简介的取法: 默认的英文页把日期写成 `Sale Day`, 简介只剩一个番号,
/// 拿不到可用的正文. 这是日文站, 就按日文取.
fn options() -> RequestOptions {
    RequestOptions::new().header("accept-language", "ja-JP,ja;q=0.9")
}

fn transport_error(error: NetError) -> SourceError {
    SourceError::Transport {
        id: ID,
        message: error.to_string(),
    }
}

fn parse_failure(message: &str, html: &str) -> SourceError {
    SourceError::parse(
        ID,
        message.to_owned(),
        RawBody {
            content: html.to_owned(),
            extension: "html",
        },
    )
}

/// 番号里的数字商品 ID. 官方站写 `FC2-PPV-<id>`, 索引站多写 `FC2-<id>`; 两种都认.
fn article_id(number: &MediaNumber) -> Option<String> {
    let raw = number.as_str();
    let rest = raw.strip_prefix("FC2-")?;
    let digits = rest.strip_prefix("PPV-").unwrap_or(rest);

    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| digits.to_owned())
}

fn parse_detail(html: &str, article_id: &str) -> Result<ScrapedMetadata, SourceError> {
    let document = Html::parse_document(html);

    // 不在售时页面是「找不到商品」, 与未收录同义; 缺别的块才算解析失败.
    let block = Selector::parse(ARTICLE_BLOCK).expect("固定选择器");
    if document.select(&block).next().is_none() {
        if is_missing_page(&document) {
            return Err(SourceError::NotFound {
                id: ID,
                number: format!("FC2-PPV-{article_id}"),
            });
        }
        return Err(parse_failure("商品页没有用户信息块", html));
    }

    // 标题取主图 alt: 标题行里除标题外还有促销角标与站点插的隐藏水印, 只有 alt 是干净的.
    let Some(title) = attrs(&document, "div.items_article_MainitemThumb img", "alt")
        .into_iter()
        .next()
    else {
        return Err(parse_failure("商品页没有标题", html));
    };

    let samples = attrs(&document, "ul.items_article_SampleImagesArea li a", "href")
        .into_iter()
        .map(|href| absolute(BASE_URL, &href))
        .collect::<Vec<_>>();
    let poster = attrs(&document, "div.items_article_MainitemThumb span img", "src")
        .into_iter()
        .next()
        .map(|src| absolute(BASE_URL, &src));

    let mut scraped = ScrapedMetadata {
        title: Some(title),
        plot: attrs(&document, r#"meta[name="description"]"#, "content")
            .into_iter()
            .next(),
        // 日期写在「販売日 : 2026/10/14」这类段落里, 标签随语言变, 因此不认标签:
        // 这几段里第一个能解析成日期的就是发售日.
        release_date: texts(&document, "div.items_article_softDevice p")
            .iter()
            .find_map(|line| find_date(line)),
        runtime_minutes: first_text(&document, "p.items_article_info")
            .as_deref()
            .and_then(runtime_of),
        maker: texts(&document, "section.items_comment_sellerBox h4 a")
            .into_iter()
            .next(),
        genres: texts(&document, "a.tag.tagTag"),
        poster_urls: poster.into_iter().collect(),
        // 样品图首张即横版封面, 其余进剧照.
        thumb_urls: samples.first().cloned().into_iter().collect(),
        ..ScrapedMetadata::default()
    };
    if !samples.is_empty() {
        scraped.extrafanart_urls.insert(ID.to_owned(), samples);
    }

    Ok(scraped)
}

/// 时长: 站点写成 `58:01` (分:秒), 长片会写成 `1:02:03` (时:分:秒).
fn runtime_of(value: &str) -> Option<i32> {
    let parts: Vec<&str> = value.trim().split(':').collect();
    let minutes = match parts.as_slice() {
        [minutes, _seconds] => minutes.parse::<i32>().ok()?,
        [hours, minutes, _seconds] => {
            hours.parse::<i32>().ok()? * 60 + minutes.parse::<i32>().ok()?
        }
        _ => return None,
    };

    (minutes > 0).then_some(minutes)
}

/// 不在售页面的标志写在标题里.
fn is_missing_page(document: &Html) -> bool {
    let Some(title) = first_text(document, "title") else {
        return false;
    };

    MISSING_MARKERS.iter().any(|marker| title.contains(marker))
}

#[cfg(test)]
mod tests {
    use amane_core::MediaNumber;

    use super::{article_id, parse_detail};

    const ARTICLE: &str = r#"
        <div class="items_article_header">
          <div class="items_article_MainitemThumb">
            <span><img src="//contents-thumbnail.fc2.com/poster.jpg" /></span>
            <img alt="商品标题" src="//contents-thumbnail.fc2.com/main.jpg" />
          </div>
          <div class="items_article_headerInfo" data-section="userInfo">标题与收藏</div>
          <section class="items_comment_sellerBox" data-section="userInfo">
            <h3>販売者情報</h3>
            <h4><a href="/users/seller/">卖家名</a></h4>
          </section>
          <p class="items_article_info">58:01</p>
          <div class="items_article_softDevice">
            <p>販売日 : 2021/03/04</p>
            <p>商品ID : FC2 PPV 1234567</p>
          </div>
          <ul class="items_article_SampleImagesArea">
            <li><a href="//contents-thumbnail.fc2.com/s1.jpg"></a></li>
            <li><a href="//contents-thumbnail.fc2.com/s2.jpg"></a></li>
          </ul>
          <a class="tag tagTag" href="/tag/1">素人</a>
          <a class="tag tagTag" href="/tag/2">企画</a>
          <meta name="description" content="官方简介" />
        </div>
    "#;

    /// 已下架: 站点回 200, 但页面是「找不到商品」.
    const WITHDRAWN: &str = r#"
        <html><head><title>お探しの商品が見つかりませんでした | FC2</title></head>
        <body><div class="items_article_MainitemThumb"></div></body></html>
    "#;

    /// 同一页面按语言给的英文标题: 浏览器指纹默认带英文语言头, 拿到的就是这一份.
    const WITHDRAWN_EN: &str = r#"
        <html><head><title>We couldn&#039;t find any products that match your search | FC2</title></head>
        <body></body></html>
    "#;

    /// 改版后的页面: 用户信息块还在, 标题没了.
    const CHANGED: &str = r#"<div data-section="userInfo">卖家信息</div>"#;

    fn number(raw: &str) -> MediaNumber {
        MediaNumber::parse(raw).expect("应解析成功")
    }

    #[test]
    fn reads_article_id_from_both_writings() {
        assert_eq!(
            article_id(&number("FC2-1234567")).as_deref(),
            Some("1234567")
        );
        assert_eq!(
            article_id(&number("FC2-PPV-1234567")).as_deref(),
            Some("1234567")
        );
        assert_eq!(article_id(&number("SSIS-001")), None, "非 FC2 番号不拼地址");
    }

    #[test]
    fn parses_article_fields() {
        let scraped = parse_detail(ARTICLE, "1234567").expect("应解析成功");

        assert_eq!(scraped.title.as_deref(), Some("商品标题"));
        assert_eq!(
            scraped.release_date.map(|date| date.to_string()),
            Some("2021-03-04".to_owned())
        );
        assert_eq!(scraped.runtime_minutes, Some(58), "时长取自「分:秒」");
        assert_eq!(scraped.maker.as_deref(), Some("卖家名"));
        assert_eq!(scraped.genres, vec!["素人".to_owned(), "企画".to_owned()]);
        assert_eq!(scraped.plot.as_deref(), Some("官方简介"));
    }

    #[test]
    fn samples_become_cover_and_gallery() {
        let scraped = parse_detail(ARTICLE, "1234567").expect("应解析成功");

        assert_eq!(
            scraped.thumb_urls.first().map(String::as_str),
            Some("https://contents-thumbnail.fc2.com/s1.jpg"),
            "样品图首张即横版封面"
        );
        assert_eq!(
            scraped.poster_urls.first().map(String::as_str),
            Some("https://contents-thumbnail.fc2.com/poster.jpg")
        );
        assert_eq!(scraped.extrafanart_urls.get("fc2").map(Vec::len), Some(2));
    }

    #[test]
    fn runtime_handles_hours_and_garbage() {
        assert_eq!(super::runtime_of("58:01"), Some(58));
        assert_eq!(super::runtime_of("1:02:03"), Some(62));
        assert_eq!(super::runtime_of("00:30"), None, "零时长不算有效值");
        assert_eq!(super::runtime_of("—"), None);
    }

    #[test]
    fn withdrawn_article_is_not_found() {
        let error = parse_detail(WITHDRAWN, "1234567").expect_err("下架应判未收录");

        match error {
            super::SourceError::NotFound { number, .. } => {
                assert_eq!(number, "FC2-PPV-1234567", "报错用官方站的番号写法");
            }
            other => panic!("应判未收录, 实际 {other:?}"),
        }
    }

    #[test]
    fn withdrawn_article_in_english_is_not_found_too() {
        let error = parse_detail(WITHDRAWN_EN, "1234567").expect_err("英文下架页也应判未收录");

        assert!(matches!(error, super::SourceError::NotFound { .. }));
    }

    #[test]
    fn changed_layout_keeps_raw() {
        let error = parse_detail(CHANGED, "1234567").expect_err("没有标题应判解析失败");

        match &error {
            super::SourceError::Parse { message, raw, .. } => {
                assert!(message.contains("标题"), "说明应指出缺什么: {message}");
                assert_eq!(raw.content, CHANGED);
                assert_eq!(raw.extension, "html");
            }
            other => panic!("应判解析失败, 实际 {other:?}"),
        }
    }
}
