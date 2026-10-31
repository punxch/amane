//! mgstage. 素人片的来源, 也覆盖它家的有码条目.
//!
//! 站内地址逐部不同地写不写开头的数字段, 因此直连与检索都按番号的多种形态逐个试.

use std::sync::Arc;

use amane_core::MediaNumber;
use amane_net::{NetError, RequestOptions, WebClient};
use async_trait::async_trait;
use scraper::{ElementRef, Html, Selector};

use crate::dom::{absolute, attrs, find_date, first_text, parse_minutes, text_of};
use crate::number::{select, variants};
use crate::{Capability, ScrapedMetadata, Source, SourceDescriptor, SourceError};

/// 请求实际打到的主机名; 限速按它分桶.
pub const HOST: &str = "www.mgstage.com";

const BASE_URL: &str = "https://www.mgstage.com";

const ID: &str = "mgstage";

/// 竖版海报与横版放大图同目录同名, 只差文件名前缀; 详情页只引用后者.
const COVER_PREFIX: &str = "pb_e_";
const POSTER_PREFIX: &str = "pf_e_";

/// 素人条目的出演栏写的是站点的代名 (名字后跟年龄与职业), 不是可对齐的演员身份.
const AMATEUR_TAG: &str = "素人";

pub struct MgstageSource {
    client: Arc<WebClient>,
}

impl MgstageSource {
    pub fn new(client: Arc<WebClient>) -> Self {
        Self { client }
    }

    async fn locate(&self, number: &MediaNumber) -> Result<Option<String>, SourceError> {
        for variant in variants(number) {
            if let Some(url) = self.locate_variant(&variant, number).await? {
                return Ok(Some(url));
            }
        }

        Ok(None)
    }

    async fn locate_variant(
        &self,
        variant: &str,
        number: &MediaNumber,
    ) -> Result<Option<String>, SourceError> {
        let direct = format!("{BASE_URL}/product/product_detail/{variant}/");
        // 直连失败 (含 404) 都退到检索: 这一家的地址形态不统一, 一次 404 说明不了没有这部片.
        if let Ok(html) = self.client.get_text_with(&direct, &options()).await
            && first_text(&Html::parse_document(&html), "#center_column h1").is_some()
        {
            return Ok(Some(direct));
        }

        let search = format!("{BASE_URL}/search/cSearch.php?search_word={variant}");
        let html = match self.client.get_text_with(&search, &options()).await {
            Ok(html) => html,
            Err(NetError::Status { status: 404, .. }) => return Ok(None),
            Err(error) => return Err(SourceError::transport(ID, &error)),
        };

        Ok(result_path(&html, number))
    }
}

#[async_trait]
impl Source for MgstageSource {
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
            .map_err(|error| SourceError::transport(ID, &error))?;

        parse_detail(&html, number)
    }
}

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: ID,
    label: "MGStage",
    base_url: BASE_URL,
    hosts: &[HOST],
    rate: amane_net::Rate::each_second(1, 1),
    capabilities: &[Capability::Metadata],
};

/// 站点要求的请求头: `adc=1` 即年龄确认, 不带它拿到的是确认页.
fn options() -> RequestOptions {
    RequestOptions::new().cookie("adc=1")
}

/// 检索结果里与请求番号同一部的地址.
fn result_path(html: &str, number: &MediaNumber) -> Option<String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse(r#"a[href*="/product/product_detail/"]"#).expect("固定选择器");

    let mut candidates: Vec<(String, String)> = Vec::new();
    for href in document
        .select(&selector)
        .filter_map(|element| element.value().attr("href"))
    {
        let url = absolute(BASE_URL, href);
        let Some(found) = detail_number(&url) else {
            continue;
        };
        if candidates.iter().any(|(seen, _)| seen == &url) {
            continue;
        }
        candidates.push((url, found));
    }

    let found: Vec<String> = candidates.iter().map(|(_, found)| found.clone()).collect();
    let index = select(&found, number.as_str())?;

    candidates.get(index).map(|(url, _)| url.clone())
}

/// 详情页地址里的番号段.
fn detail_number(url: &str) -> Option<String> {
    let rest = url.split("/product/product_detail/").nth(1)?;
    let number = rest.split('/').next()?.trim();

    (!number.is_empty()).then(|| number.to_owned())
}

/// 详情页表格里某一行的取值单元格.
fn field_cell<'a>(document: &'a Html, label: &str) -> Option<ElementRef<'a>> {
    let header_selector = Selector::parse("th").expect("固定选择器");
    let cell_selector = Selector::parse("td").expect("固定选择器");

    for header in document.select(&header_selector) {
        if !text_of(&header).contains(label) {
            continue;
        }
        let Some(row) = header.parent().and_then(ElementRef::wrap) else {
            continue;
        };
        return row.select(&cell_selector).next();
    }

    None
}

/// 某一行里的链接文本, 如题材、出演、厂牌.
fn field_links(document: &Html, label: &str) -> Vec<String> {
    let Some(cell) = field_cell(document, label) else {
        return Vec::new();
    };
    let selector = Selector::parse("a").expect("固定选择器");

    cell.select(&selector)
        .map(|link| text_of(&link))
        .filter(|text| !text.is_empty())
        .collect()
}

/// 某一行里的纯文本, 如发行日期、时长.
fn field_text(document: &Html, label: &str) -> Option<String> {
    field_cell(document, label).and_then(|cell| {
        let text = text_of(&cell);
        (!text.is_empty()).then_some(text)
    })
}

fn parse_detail(html: &str, number: &MediaNumber) -> Result<ScrapedMetadata, SourceError> {
    let document = Html::parse_document(html);

    let Some(title) = first_text(&document, "#center_column h1") else {
        return Err(SourceError::unparsable(ID, "详情页没有标题", html));
    };
    // 表格是这一家的全部字段来源; 连表格都没有说明拿到了确认页或改版页.
    let Some(identifier) = field_text(&document, "品番") else {
        return Err(SourceError::unparsable(ID, "详情页没有商品信息表", html));
    };
    if !identifier
        .replace(' ', "")
        .eq_ignore_ascii_case(number.as_str())
    {
        tracing::warn!(number = %number, found = %identifier, "mgstage 详情页番号与请求不符");
        return Err(SourceError::NotFound {
            id: ID,
            number: number.as_str().to_owned(),
        });
    }

    let genres = field_links(&document, "ジャンル");
    // 素人条目的出演值不可对齐, 写进演员字段会顶掉 javdb 记的本人 (演员按首个非空来源锁定),
    // 因此这一家对素人条目**故意不给出演者**. 这是取舍, 不是遗漏.
    let amateur = genres.iter().any(|genre| genre.contains(AMATEUR_TAG));
    let actors = if amateur {
        Vec::new()
    } else {
        field_links(&document, "出演")
    };

    let cover = attrs(&document, "a#EnlargeImage", "href")
        .into_iter()
        .next()
        .or_else(|| {
            attrs(&document, "img.enlarge_image", "src")
                .into_iter()
                .next()
        })
        .map(|href| absolute(BASE_URL, &href));
    let poster = cover.as_deref().and_then(poster_of);
    let extrafanart = attrs(&document, "dl#sample-photo li a", "href")
        .into_iter()
        .map(|href| absolute(BASE_URL, &href))
        .collect::<Vec<_>>();

    let mut scraped = ScrapedMetadata {
        title: Some(title),
        plot: plot_of(&document),
        release_date: field_text(&document, "配信開始日")
            .as_deref()
            .and_then(find_date),
        runtime_minutes: field_text(&document, "収録時間")
            .as_deref()
            .and_then(parse_minutes),
        maker: field_text(&document, "メーカー"),
        label: field_text(&document, "レーベル"),
        series: field_text(&document, "シリーズ"),
        actors,
        genres,
        poster_urls: poster.into_iter().collect(),
        thumb_urls: cover.into_iter().collect(),
        ..ScrapedMetadata::default()
    };
    if !extrafanart.is_empty() {
        scraped.extrafanart_urls.insert(ID.to_owned(), extrafanart);
    }

    Ok(scraped)
}

/// 商品介绍正文.
///
/// 「すべてを見る」那条链接所在段不是正文, 排除它; 其余文本逐段接起来.
fn plot_of(document: &Html) -> Option<String> {
    let selector = Selector::parse("dl#introduction dd p").expect("固定选择器");

    let paragraphs: Vec<String> = document
        .select(&selector)
        .filter(|element| {
            !element
                .value()
                .attr("class")
                .is_some_and(|class| class.contains("more"))
        })
        .map(|element| text_of(&element))
        .filter(|text| !text.is_empty())
        .collect();

    (!paragraphs.is_empty()).then(|| paragraphs.join("\n"))
}

/// 横版放大图地址 → 同目录竖版海报地址; 非 `pb_e_` 形态返回 `None`.
fn poster_of(cover: &str) -> Option<String> {
    let (directory, filename) = cover.rsplit_once('/')?;
    let rest = filename.strip_prefix(COVER_PREFIX)?;

    Some(format!("{directory}/{POSTER_PREFIX}{rest}"))
}

#[cfg(test)]
mod tests {
    use amane_core::MediaNumber;

    use super::{detail_number, parse_detail, poster_of, result_path};

    const SEARCH: &str = r#"
        <div class="search_result">
          <a href="/product/product_detail/SSIS-002/">别的片</a>
          <a href="/product/product_detail/SSIS-001/">目标</a>
          <a href="/product/product_detail/SSIS-001/">重复</a>
        </div>
    "#;

    /// 素人条目: 出演栏是站点代名, 题材里有「素人」.
    const AMATEUR: &str = r#"
        <div id="center_column">
          <div class="common_frame">
            <h1>素人作品标题</h1>
            <dl id="introduction">
              <dd><p class="txt introduction">第一段介绍</p><p class="txt introduction">第二段介绍</p>
                  <p class="more"><a>すべてを見る</a></p></dd>
            </dl>
            <table class="data">
              <tr><th>品番：</th><td>SIRO-1234</td></tr>
              <tr><th>配信開始日：</th><td>2020/12/25</td></tr>
              <tr><th>収録時間：</th><td>120分</td></tr>
              <tr><th>メーカー：</th><td><a href="/maker/1">素人メーカー</a></td></tr>
              <tr><th>レーベル：</th><td><a href="/label/1">レーベル名</a></td></tr>
              <tr><th>シリーズ：</th><td><a href="/series/1">シリーズ名</a></td></tr>
              <tr><th>ジャンル：</th><td><a href="/genre/1">素人</a><a href="/genre/2">ドキュメント</a></td></tr>
              <tr><th>出演：</th><td><a href="/actor/1">あや 23歳 大学生</a></td></tr>
            </table>
            <a id="EnlargeImage" href="https://www.mgstage.com/images/pb_e_abc.jpg"></a>
            <dl id="sample-photo"><li><a href="/sample/1.jpg"></a><a href="/sample/2.jpg"></a></li></dl>
          </div>
        </div>
    "#;

    /// 有码条目: 出演是可对齐的演员.
    const CENSORED: &str = r#"
        <div id="center_column">
          <div class="common_frame">
            <h1>有码作品标题</h1>
            <table class="data">
              <tr><th>品番：</th><td>SSIS-001</td></tr>
              <tr><th>ジャンル：</th><td><a href="/genre/1">ドラマ</a></td></tr>
              <tr><th>出演：</th><td><a href="/actor/1">葵つかさ</a></td></tr>
            </table>
          </div>
        </div>
    "#;

    /// 改版后的页面: 标题还在, 商品信息表没了.
    const CHANGED: &str = r#"<div id="center_column"><h1>标题</h1><p>改版了</p></div>"#;

    fn number(raw: &str) -> MediaNumber {
        MediaNumber::parse(raw).expect("应解析成功")
    }

    #[test]
    fn takes_the_result_with_matching_number() {
        assert_eq!(
            result_path(SEARCH, &number("SSIS-001")).as_deref(),
            Some("https://www.mgstage.com/product/product_detail/SSIS-001/"),
            "检索结果不取首条, 按番号比对"
        );
    }

    #[test]
    fn unknown_number_is_not_found() {
        assert_eq!(
            result_path(SEARCH, &number("ABP-123")),
            None,
            "没有同一部的结果即未收录"
        );
    }

    #[test]
    fn reads_number_from_detail_path() {
        assert_eq!(
            detail_number("https://www.mgstage.com/product/product_detail/SIRO-1234/").as_deref(),
            Some("SIRO-1234")
        );
        assert_eq!(detail_number("https://www.mgstage.com/search/x"), None);
    }

    #[test]
    fn amateur_entries_omit_actors() {
        let scraped = parse_detail(AMATEUR, &number("SIRO-1234")).expect("应解析成功");

        assert!(
            scraped.actors.is_empty(),
            "素人条目的出演值是站点代名, 不能顶掉 javdb 的本人"
        );
        assert_eq!(
            scraped.genres,
            vec!["素人".to_owned(), "ドキュメント".to_owned()]
        );
    }

    #[test]
    fn censored_entries_keep_actors() {
        let scraped = parse_detail(CENSORED, &number("SSIS-001")).expect("应解析成功");

        assert_eq!(scraped.actors, vec!["葵つかさ".to_owned()]);
    }

    #[test]
    fn parses_table_fields() {
        let scraped = parse_detail(AMATEUR, &number("SIRO-1234")).expect("应解析成功");

        assert_eq!(scraped.title.as_deref(), Some("素人作品标题"));
        assert_eq!(
            scraped.release_date.map(|date| date.to_string()),
            Some("2020-12-25".to_owned())
        );
        assert_eq!(scraped.runtime_minutes, Some(120));
        assert_eq!(scraped.maker.as_deref(), Some("素人メーカー"));
        assert_eq!(scraped.label.as_deref(), Some("レーベル名"));
        assert_eq!(scraped.series.as_deref(), Some("シリーズ名"));
        assert_eq!(
            scraped.plot.as_deref(),
            Some("第一段介绍\n第二段介绍"),
            "「すべてを見る」不算正文"
        );
    }

    #[test]
    fn derives_poster_from_cover_prefix() {
        assert_eq!(
            poster_of("https://www.mgstage.com/images/pb_e_abc.jpg").as_deref(),
            Some("https://www.mgstage.com/images/pf_e_abc.jpg")
        );
        assert_eq!(
            poster_of("https://www.mgstage.com/images/other.jpg"),
            None,
            "不是 pb_e_ 形态时不猜"
        );
    }

    #[test]
    fn keeps_cover_and_samples() {
        let scraped = parse_detail(AMATEUR, &number("SIRO-1234")).expect("应解析成功");

        assert_eq!(
            scraped.thumb_urls.first().map(String::as_str),
            Some("https://www.mgstage.com/images/pb_e_abc.jpg")
        );
        assert_eq!(
            scraped.poster_urls.first().map(String::as_str),
            Some("https://www.mgstage.com/images/pf_e_abc.jpg")
        );
        assert_eq!(
            scraped.extrafanart_urls.get("mgstage").map(Vec::len),
            Some(2)
        );
    }

    #[test]
    fn changed_layout_keeps_raw() {
        let error = parse_detail(CHANGED, &number("SSIS-001")).expect_err("没有信息表应判解析失败");

        match &error {
            super::SourceError::Parse { message, raw, .. } => {
                assert!(message.contains("信息表"), "说明应指出缺什么: {message}");
                assert_eq!(raw.content, CHANGED);
                assert_eq!(raw.extension, "html");
            }
            other => panic!("应判解析失败, 实际 {other:?}"),
        }
    }
}
