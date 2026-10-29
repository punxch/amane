//! javbus. 先按番号直连详情页, 未命中再回退站内搜索.
//!
//! 站内地址直接就是番号 (`/SSIS-001`), 因此多数情况下省掉一次检索请求. 检索是模糊匹配,
//! 结果里会混进番号相近的条目, 所以逐条比对番号而不是取首条.

use std::sync::Arc;

use amane_core::MediaNumber;
use amane_net::{NetError, RequestOptions, WebClient};
use async_trait::async_trait;
use scraper::{ElementRef, Html, Selector};

use crate::dom::{absolute, attrs, find_date, first_text, parse_minutes, text_of, texts};
use crate::number::{select, variants};
use crate::{Capability, RawBody, ScrapedMetadata, Source, SourceDescriptor, SourceError};

/// 请求实际打到的主机名; 限速按它分桶.
pub const HOST: &str = "www.javbus.com";

const BASE_URL: &str = "https://www.javbus.com";

const ID: &str = "javbus";

pub struct JavbusSource {
    client: Arc<WebClient>,
}

impl JavbusSource {
    pub fn new(client: Arc<WebClient>) -> Self {
        Self { client }
    }

    /// 定位详情页; 站内没有这部片时返回 `None`.
    async fn locate(&self, number: &MediaNumber) -> Result<Option<String>, SourceError> {
        let mut last_error = None;
        for variant in variants(number) {
            match self.locate_variant(&variant, number).await {
                Ok(Some(url)) => return Ok(Some(url)),
                Ok(None) => {}
                Err(error) => last_error = Some(error),
            }
        }

        match last_error {
            Some(error) => Err(error),
            None => Ok(None),
        }
    }

    async fn locate_variant(
        &self,
        variant: &str,
        number: &MediaNumber,
    ) -> Result<Option<String>, SourceError> {
        // 直连优先: 命中率高, 且省掉一次检索. 直连 404 与检索 404 同义, 都只是「没这部片」.
        let direct = format!("{BASE_URL}/{variant}");
        if let Ok(html) = self.client.get_text_with(&direct, &options()).await
            && is_movie_page(&html)
        {
            return Ok(Some(direct));
        }

        let search = format!("{BASE_URL}/search/{variant}&type=&parent=ce");
        let html = match self.client.get_text_with(&search, &options()).await {
            Ok(html) => html,
            // 检索词一条也匹配不上时站点直接 404; 报成请求失败会让人去查网络.
            Err(NetError::Status { status: 404, .. }) => return Ok(None),
            Err(error) => return Err(transport_error(error)),
        };

        Ok(first_result_path(&html, number).map(|path| absolute(BASE_URL, &path)))
    }
}

#[async_trait]
impl Source for JavbusSource {
    fn descriptor(&self) -> &SourceDescriptor {
        &DESCRIPTOR
    }

    async fn fetch(&self, number: &MediaNumber) -> Result<ScrapedMetadata, SourceError> {
        let Some(url) = self.locate(number).await? else {
            return Err(SourceError::NotFound {
                id: ID,
                number: number.as_str().to_owned(),
            });
        };

        let html = self
            .client
            .get_text_with(&url, &options())
            .await
            .map_err(transport_error)?;

        parse_detail(&html, number)
    }
}

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: ID,
    label: "JavBus",
    base_url: BASE_URL,
    host: HOST,
    // 站点没有公开的速率要求; 一次刮削只发两三个请求, 一秒一个足够慢.
    rate: amane_net::Rate::each_second(1, 1),
    capabilities: &[Capability::Metadata],
};

/// 站点要求的请求头.
///
/// `dv=1` 关掉年龄确认遮罩, 否则拿到的是确认页而不是影片页; 语言头决定面板用中文还是英文,
/// 两套标签都要认, 但优先拿中文, 标题与题材才有中文写法.
fn options() -> RequestOptions {
    RequestOptions::new().cookie("dv=1").header(
        "accept-language",
        "zh-CN,zh;q=0.9,en;q=0.8,en-GB;q=0.7,en-US;q=0.6",
    )
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

/// 影片页的标志: 有 `movie-box` 区块且有标题.
///
/// 只认其中一个都不够: 年龄确认页与站点首页都没有 `movie-box`, 而改版后的空壳页可能只有区块名.
fn is_movie_page(html: &str) -> bool {
    html.contains("movie-box") && first_text(&Html::parse_document(html), "h3").is_some()
}

/// 搜索结果里与请求番号同一部的路径.
fn first_result_path(html: &str, number: &MediaNumber) -> Option<String> {
    let document = Html::parse_document(html);
    let item_selector = Selector::parse("a.movie-box").expect("固定选择器");
    let date_selector = Selector::parse("div.photo-info date").expect("固定选择器");

    let mut candidates = Vec::new();
    for item in document.select(&item_selector) {
        let Some(href) = item
            .value()
            .attr("href")
            .map(str::trim)
            .filter(|href| !href.is_empty())
        else {
            continue;
        };
        let found = item
            .select(&date_selector)
            .next()
            .map(|element| text_of(&element))
            .unwrap_or_default();
        candidates.push((href.to_owned(), found));
    }

    let found: Vec<String> = candidates.iter().map(|(_, found)| found.clone()).collect();
    let index = select(&found, number.as_str())?;

    candidates.get(index).map(|(href, _)| href.clone())
}

/// 详情页的「标签 + 值」.
///
/// 站点把标签写在 `span.header` 里, 值跟在同一个 `<p>` 中; 取整个 `<p>` 的文本再削掉标签,
/// 比顺着兄弟节点取值更耐改版 (值的节点形态各字段并不一致).
fn labeled_value(document: &Html, label: &str) -> Option<String> {
    let selector = Selector::parse("span.header").expect("固定选择器");

    for header in document.select(&selector) {
        let header_text = text_of(&header);
        if !header_text.contains(label) {
            continue;
        }
        let Some(row) = header.parent().and_then(ElementRef::wrap) else {
            continue;
        };
        let row_text = text_of(&row);
        let value = row_text
            .strip_prefix(&header_text)
            .unwrap_or(&row_text)
            .trim()
            .trim_start_matches([':', '：'])
            .trim();

        return Some(value.to_owned());
    }

    None
}

fn parse_detail(html: &str, number: &MediaNumber) -> Result<ScrapedMetadata, SourceError> {
    let document = Html::parse_document(html);

    let Some(title) = first_text(&document, "h3") else {
        return Err(parse_failure("详情页没有标题", html));
    };
    let Some(identifier) = labeled_value(&document, "識別碼") else {
        return Err(parse_failure("详情页没有識別碼", html));
    };
    if !identifier
        .replace(' ', "")
        .eq_ignore_ascii_case(number.as_str())
    {
        tracing::warn!(number = %number, found = %identifier, "javbus 详情页番号与请求不符");
        return Err(SourceError::NotFound {
            id: ID,
            number: number.as_str().to_owned(),
        });
    }

    // 站点的横版封面写在 `a.bigImage` 里; 竖版海报同目录同名, 只差路径与文件名后缀.
    let cover = attrs(&document, "a.bigImage", "href")
        .into_iter()
        .next()
        .map(|href| absolute(BASE_URL, &href));
    let poster = cover.as_deref().and_then(poster_of);
    let extrafanart = attrs(&document, "div#sample-waterfall a", "href")
        .into_iter()
        .map(|href| absolute(BASE_URL, &href))
        .collect::<Vec<_>>();

    let mut scraped = ScrapedMetadata {
        title: Some(title),
        release_date: labeled_value(&document, "發行日期")
            .as_deref()
            .and_then(find_date),
        runtime_minutes: labeled_value(&document, "長度")
            .as_deref()
            .and_then(parse_minutes),
        maker: labeled_value(&document, "片商"),
        label: labeled_value(&document, "發行商"),
        series: labeled_value(&document, "系列"),
        directors: labeled_value(&document, "導演").into_iter().collect(),
        actors: texts(&document, "div.star-name a"),
        genres: texts(&document, "span.genre label a"),
        poster_urls: poster.into_iter().collect(),
        thumb_urls: cover.into_iter().collect(),
        ..ScrapedMetadata::default()
    };
    if !extrafanart.is_empty() {
        scraped.extrafanart_urls.insert(ID.to_owned(), extrafanart);
    }

    Ok(scraped)
}

/// 横版封面地址 → 竖版海报地址.
///
/// 站点约定: 海报在 `/thumb/` 下且不带 `_b` 后缀, 封面在 `/cover/` 下且带 `_b`.
/// 详情页只引用封面, 海报地址只能这样推出来.
fn poster_of(cover: &str) -> Option<String> {
    let poster = cover
        .replace("/cover/", "/thumb/")
        .replace("_b.jpg", ".jpg");

    (poster != cover).then_some(poster)
}

#[cfg(test)]
mod tests {
    use amane_core::MediaNumber;

    use super::{first_result_path, is_movie_page, parse_detail, poster_of};

    const SEARCH: &str = r#"
        <div id="waterfall">
          <a class="movie-box" href="https://www.javbus.com/SSIS-002">
            <div class="photo-info"><date>SSIS-002</date><span>别的片</span></div>
          </a>
          <a class="movie-box" href="https://www.javbus.com/SSIS-001">
            <div class="photo-info"><date>SSIS-001</date><span>目标</span></div>
          </a>
        </div>
    "#;

    const DETAIL: &str = r#"
        <div class="container movie-box">
          <h3>中文标题</h3>
          <div class="row movie">
            <div class="col-md-9 screencap">
              <a class="bigImage" href="https://www.javbus.com/cover/abc_b.jpg">
                <img src="https://www.javbus.com/cover/abc_b.jpg" />
              </a>
            </div>
            <div class="col-md-3 info">
              <p><span class="header">識別碼:</span> <span style="color:#CC0000;">SSIS-001</span></p>
              <p><span class="header">發行日期:</span> 2021-02-19</p>
              <p><span class="header">長度:</span> 150分鐘</p>
              <p><span class="header">導演:</span> <a href="/director/abc">粟原</a></p>
              <p><span class="header">片商:</span> <a href="/studio/abc">エスワン</a></p>
              <p><span class="header">發行商:</span> <a href="/label/abc">S1 NO.1 STYLE</a></p>
              <p><span class="header">系列:</span> <a href="/series/abc">系列名</a></p>
              <p class="star-show">
                <span class="genre"><label><a href="/genre/1">ドラマ</a></label></span>
              </p>
            </div>
          </div>
          <div class="star-name"><a href="/star/abc">葵つかさ</a><a href="/star/def">乙白さやか</a></div>
          <div id="sample-waterfall">
            <a href="https://www.javbus.com/pics/sample/1.jpg"></a>
            <a href="https://www.javbus.com/pics/sample/2.jpg"></a>
          </div>
        </div>
    "#;

    /// 年龄确认页: 有站点外壳, 没有影片区块.
    const AGE_GATE: &str =
        r#"<html><body><div class="container"><p>是否已满 18 岁</p></div></body></html>"#;

    /// 改版后的详情页: 外壳还在, 标签节点没了.
    const CHANGED: &str = r#"<html><body><h3>标题</h3><div class="movie-box"></div></body></html>"#;

    fn number() -> MediaNumber {
        MediaNumber::parse("SSIS-001").expect("应解析成功")
    }

    #[test]
    fn takes_the_result_with_matching_number() {
        assert_eq!(
            first_result_path(SEARCH, &number()).as_deref(),
            Some("https://www.javbus.com/SSIS-001"),
            "检索是模糊匹配, 不能取首条"
        );
    }

    #[test]
    fn empty_result_page_is_not_found() {
        let empty = r#"<div id="waterfall"></div>"#;

        assert_eq!(
            first_result_path(empty, &number()),
            None,
            "没有结果即未收录, 不是解析失败"
        );
    }

    #[test]
    fn age_gate_is_not_a_movie_page() {
        assert!(!is_movie_page(AGE_GATE), "确认页不能当成影片页");
        assert!(is_movie_page(DETAIL));
    }

    #[test]
    fn parses_detail_panel() {
        let scraped = parse_detail(DETAIL, &number()).expect("应解析成功");

        assert_eq!(scraped.title.as_deref(), Some("中文标题"));
        assert_eq!(
            scraped.release_date.map(|date| date.to_string()),
            Some("2021-02-19".to_owned())
        );
        assert_eq!(scraped.runtime_minutes, Some(150));
        assert_eq!(scraped.maker.as_deref(), Some("エスワン"));
        assert_eq!(scraped.label.as_deref(), Some("S1 NO.1 STYLE"));
        assert_eq!(scraped.series.as_deref(), Some("系列名"));
        assert_eq!(scraped.directors, vec!["粟原".to_owned()]);
        assert_eq!(scraped.actors, vec!["葵つかさ", "乙白さやか"]);
        assert_eq!(scraped.genres, vec!["ドラマ".to_owned()]);
    }

    #[test]
    fn rewrites_cover_into_poster() {
        assert_eq!(
            poster_of("https://www.javbus.com/cover/abc_b.jpg").as_deref(),
            Some("https://www.javbus.com/thumb/abc.jpg"),
            "海报按站点约定由封面地址推出"
        );
        assert_eq!(
            poster_of("https://www.javbus.com/pics/abc.jpg"),
            None,
            "推不出时宁可不给, 也不能把封面当海报"
        );
    }

    #[test]
    fn keeps_cover_and_samples() {
        let scraped = parse_detail(DETAIL, &number()).expect("应解析成功");

        assert_eq!(
            scraped.thumb_urls.first().map(String::as_str),
            Some("https://www.javbus.com/cover/abc_b.jpg")
        );
        assert_eq!(
            scraped.poster_urls.first().map(String::as_str),
            Some("https://www.javbus.com/thumb/abc.jpg")
        );
        assert_eq!(
            scraped.extrafanart_urls.get("javbus").map(Vec::len),
            Some(2),
            "剧照按站点分组"
        );
    }

    #[test]
    fn changed_layout_keeps_raw() {
        let error = parse_detail(CHANGED, &number()).expect_err("没有識別碼应判解析失败");

        match &error {
            super::SourceError::Parse { message, raw, .. } => {
                assert!(message.contains("識別碼"), "说明应指出缺什么: {message}");
                assert_eq!(raw.content, CHANGED, "应保留完整原文");
                assert_eq!(raw.extension, "html");
            }
            other => panic!("应判解析失败, 实际 {other:?}"),
        }
    }

    #[test]
    fn rejects_page_for_another_number() {
        let other = MediaNumber::parse("ABC-999").expect("应解析成功");

        let result = parse_detail(DETAIL, &other);

        assert!(
            matches!(result, Err(super::SourceError::NotFound { .. })),
            "番号不符时必须报未收录"
        );
    }
}
