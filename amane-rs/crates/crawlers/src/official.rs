//! 片商官网. 按番号的系列前缀路由到对应的片商站点.
//!
//! 这些站点共用同一套 CMS (Outvision / Will 集团), 详情页结构一致, 因此只有一个解析器;
//! 差异只在域名与番号前缀, 两者都由表驱动.
//!
//! 路由只看番号的系列前缀: 按前序来源的片商名反查需要聚合结果, 而 `Source::fetch`
//! 只拿得到番号. 前缀表覆盖不到的新系列要靠后续补表, 见 `docs/adr/0008`.

use std::sync::Arc;

use amane_core::MediaNumber;
use amane_net::{NetError, RequestOptions, WebClient};
use async_trait::async_trait;
use scraper::{CaseSensitivity, ElementRef, Html, Selector};

use crate::dom::{absolute, attrs, find_date, own_text_of, parse_minutes, text_of};
use crate::{Capability, ScrapedMetadata, Source, SourceDescriptor, SourceError};

const ID: &str = "official";

/// 一个片商与其官网.
struct Manufacturer {
    /// 来源内部的标识, 路由表用它指认片商.
    id: &'static str,
    domain: &'static str,
}

const MANUFACTURERS: &[Manufacturer] = &[
    Manufacturer {
        id: "attackers",
        domain: "attackers.net",
    },
    Manufacturer {
        id: "befree",
        domain: "befreebe.com",
    },
    Manufacturer {
        id: "bi",
        domain: "bi-av.com",
    },
    Manufacturer {
        id: "bibian",
        domain: "bibian-av.com",
    },
    Manufacturer {
        id: "das",
        domain: "dasdas.jp",
    },
    Manufacturer {
        id: "ebody",
        domain: "www.av-e-body.com",
    },
    Manufacturer {
        id: "fitch",
        domain: "fitch-av.com",
    },
    Manufacturer {
        id: "hajime_kikaku",
        domain: "hajimekikaku.com",
    },
    Manufacturer {
        id: "honnaka",
        domain: "honnaka.jp",
    },
    Manufacturer {
        id: "hunter",
        domain: "hhh-av.com",
    },
    Manufacturer {
        id: "idea_pocket",
        domain: "ideapocket.com",
    },
    Manufacturer {
        id: "kawaii",
        domain: "kawaiikawaii.jp",
    },
    Manufacturer {
        id: "kirakira",
        domain: "kirakira-av.com",
    },
    Manufacturer {
        id: "madonna",
        domain: "www.madonna-av.com",
    },
    Manufacturer {
        id: "miman",
        domain: "miman.jp",
    },
    Manufacturer {
        id: "mko_labo",
        domain: "mko-labo.net",
    },
    Manufacturer {
        id: "moodyz",
        domain: "moodyz.com",
    },
    Manufacturer {
        id: "muku",
        domain: "muku.tv",
    },
    Manufacturer {
        id: "mvg",
        domain: "mvg.jp",
    },
    Manufacturer {
        id: "nanpa_japan",
        domain: "nanpa-japan.jp",
    },
    Manufacturer {
        id: "oppai",
        domain: "oppai-av.com",
    },
    Manufacturer {
        id: "opera",
        domain: "av-opera.jp",
    },
    Manufacturer {
        id: "premium_beauty",
        domain: "premium-beauty.com",
    },
    Manufacturer {
        id: "rookie",
        domain: "rookie-av.jp",
    },
    Manufacturer {
        id: "s1",
        domain: "s1s1s1.com",
    },
    Manufacturer {
        id: "tameikegoro",
        domain: "tameikegoro.jp",
    },
    Manufacturer {
        id: "tosatsu",
        domain: "to-satsu.com",
    },
    Manufacturer {
        id: "v",
        domain: "v-av.com",
    },
    Manufacturer {
        id: "wanz_factory",
        domain: "www.wanz-factory.com",
    },
];

/// 系列前缀 → 制作商标识. 由番号的前缀段决定去哪一家.
const SERIES: &[(&str, &str)] = &[
    ("sivr", "s1"),
    ("ssis", "s1"),
    ("ssni", "s1"),
    ("snis", "s1"),
    ("soe", "s1"),
    ("oned", "s1"),
    ("one", "s1"),
    ("onsd", "s1"),
    ("ofje", "s1"),
    ("sps", "s1"),
    ("tksoe", "s1"),
    ("mdvr", "moodyz"),
    ("midv", "moodyz"),
    ("mide", "moodyz"),
    ("midd", "moodyz"),
    ("mibd", "moodyz"),
    ("mimk", "moodyz"),
    ("miid", "moodyz"),
    ("migd", "moodyz"),
    ("mifd", "moodyz"),
    ("miae", "moodyz"),
    ("miad", "moodyz"),
    ("miaa", "moodyz"),
    ("mdl", "moodyz"),
    ("mdj", "moodyz"),
    ("mdi", "moodyz"),
    ("mdg", "moodyz"),
    ("mdf", "moodyz"),
    ("mde", "moodyz"),
    ("mdld", "moodyz"),
    ("mded", "moodyz"),
    ("mizd", "moodyz"),
    ("mird", "moodyz"),
    ("mdjd", "moodyz"),
    ("rmid", "moodyz"),
    ("mdid", "moodyz"),
    ("mdmd", "moodyz"),
    ("mimu", "moodyz"),
    ("mdpd", "moodyz"),
    ("mivd", "moodyz"),
    ("mdud", "moodyz"),
    ("mdgd", "moodyz"),
    ("mdvd", "moodyz"),
    ("mias", "moodyz"),
    ("miqd", "moodyz"),
    ("mint", "moodyz"),
    ("rmpd", "moodyz"),
    ("mdrd", "moodyz"),
    ("tkmide", "moodyz"),
    ("tkmidd", "moodyz"),
    ("kmide", "moodyz"),
    ("tkmigd", "moodyz"),
    ("mdfd", "moodyz"),
    ("rmwd", "moodyz"),
    ("miab", "moodyz"),
    ("juvr", "madonna"),
    ("jusd", "madonna"),
    ("juq", "madonna"),
    ("juy", "madonna"),
    ("jux", "madonna"),
    ("jul", "madonna"),
    ("juk", "madonna"),
    ("juc", "madonna"),
    ("jukd", "madonna"),
    ("oba", "madonna"),
    ("roeb", "madonna"),
    ("roe", "madonna"),
    ("ure", "madonna"),
    ("mdon", "madonna"),
    ("obe", "madonna"),
    ("jums", "madonna"),
    ("wavr", "wanz_factory"),
    ("waaa", "wanz_factory"),
    ("bmw", "wanz_factory"),
    ("wanz", "wanz_factory"),
    ("ipvr", "idea_pocket"),
    ("ipx", "idea_pocket"),
    ("ipz", "idea_pocket"),
    ("iptd", "idea_pocket"),
    ("ipsd", "idea_pocket"),
    ("idbd", "idea_pocket"),
    ("supd", "idea_pocket"),
    ("ipit", "idea_pocket"),
    ("and", "idea_pocket"),
    ("hpd", "idea_pocket"),
    ("tkipz", "idea_pocket"),
    ("ipzz", "idea_pocket"),
    ("cosd", "idea_pocket"),
    ("anpd", "idea_pocket"),
    ("dan", "idea_pocket"),
    ("alad", "idea_pocket"),
    ("kipx", "idea_pocket"),
    ("kivr", "kirakira"),
    ("blk", "kirakira"),
    ("kibd", "kirakira"),
    ("kifd", "kirakira"),
    ("kird", "kirakira"),
    ("kisd", "kirakira"),
    ("set", "kirakira"),
    ("ebvr", "ebody"),
    ("ebod", "ebody"),
    ("mkck", "ebody"),
    ("eyan", "ebody"),
    ("cjvr", "bi"),
    ("cjod", "bi"),
    ("bbi", "bi"),
    ("bib", "bi"),
    ("cjob", "bi"),
    ("beb", "bi"),
    ("bid", "bi"),
    ("bist", "bi"),
    ("bwb", "bi"),
    ("prvr", "premium_beauty"),
    ("pgd", "premium_beauty"),
    ("pred", "premium_beauty"),
    ("pbd", "premium_beauty"),
    ("pjd", "premium_beauty"),
    ("prtd", "premium_beauty"),
    ("pxd", "premium_beauty"),
    ("pid", "premium_beauty"),
    ("ptv", "premium_beauty"),
    ("mmvr", "miman"),
    ("mmnd", "miman"),
    ("mmxd", "miman"),
    ("aom", "miman"),
    ("mevr", "tameikegoro"),
    ("meyd", "tameikegoro"),
    ("mbyd", "tameikegoro"),
    ("mdyd", "tameikegoro"),
    ("mnyd", "tameikegoro"),
    ("fcvr", "fitch"),
    ("jufe", "fitch"),
    ("jufd", "fitch"),
    ("jfb", "fitch"),
    ("juny", "fitch"),
    ("nyb", "fitch"),
    ("finh", "fitch"),
    ("gcf", "fitch"),
    ("nima", "fitch"),
    ("kavr", "kawaii"),
    ("cawd", "kawaii"),
    ("kwbd", "kawaii"),
    ("kawd", "kawaii"),
    ("kwsr", "kawaii"),
    ("kwsd", "kawaii"),
    ("kane", "kawaii"),
    ("bf", "befree"),
    ("mucd", "muku"),
    ("mudr", "muku"),
    ("mukd", "muku"),
    ("smcd", "muku"),
    ("mukc", "muku"),
    ("atvr", "attackers"),
    ("rbk", "attackers"),
    ("rbd", "attackers"),
    ("same", "attackers"),
    ("shkd", "attackers"),
    ("atid", "attackers"),
    ("adn", "attackers"),
    ("atkd", "attackers"),
    ("jbd", "attackers"),
    ("sspd", "attackers"),
    ("atad", "attackers"),
    ("azsd", "attackers"),
    ("mvr", "mko_labo"),
    ("mism", "mko_labo"),
    ("emlb", "mko_labo"),
    ("dsvr", "das"),
    ("dass", "das"),
    ("dazd", "das"),
    ("dasd", "das"),
    ("pla", "das"),
    ("mvbd", "mvg"),
    ("mvsd", "mvg"),
    ("opvr", "opera"),
    ("opbd", "opera"),
    ("opud", "opera"),
    ("ppvr", "oppai"),
    ("pppe", "oppai"),
    ("ppbd", "oppai"),
    ("pppd", "oppai"),
    ("ppsd", "oppai"),
    ("ppfd", "oppai"),
    ("vvvd", "v"),
    ("vicd", "v"),
    ("vizd", "v"),
    ("vspd", "v"),
    ("clvr", "tosatsu"),
    ("stol", "tosatsu"),
    ("club", "tosatsu"),
    ("bbvr", "bibian"),
    ("bban", "bibian"),
    ("hnvr", "honnaka"),
    ("hmn", "honnaka"),
    ("hndb", "honnaka"),
    ("hnd", "honnaka"),
    ("krnd", "honnaka"),
    ("hnky", "honnaka"),
    ("hnjc", "honnaka"),
    ("hnse", "honnaka"),
    ("rvr", "rookie"),
    ("rbb", "rookie"),
    ("rki", "rookie"),
    ("njvr", "nanpa_japan"),
    ("nnpj", "nanpa_japan"),
    ("npjb", "nanpa_japan"),
    ("hjbb", "hajime_kikaku"),
    ("hjmo", "hajime_kikaku"),
    ("avgl", "hajime_kikaku"),
    ("huntb", "hunter"),
    ("hunta", "hunter"),
    ("hunt", "hunter"),
    ("hunbl", "hunter"),
    ("royd", "hunter"),
    ("tysf", "hunter"),
];

/// 全部域名, 由片商表派生. 限速表按每台主机各建一个桶.
const HOSTS: [&str; MANUFACTURERS.len()] = {
    let mut hosts = [""; MANUFACTURERS.len()];
    let mut index = 0;
    while index < MANUFACTURERS.len() {
        hosts[index] = MANUFACTURERS[index].domain;
        index += 1;
    }
    hosts
};

pub struct OfficialSource {
    client: Arc<WebClient>,
}

impl OfficialSource {
    pub fn new(client: Arc<WebClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Source for OfficialSource {
    fn descriptor(&self) -> &SourceDescriptor {
        &DESCRIPTOR
    }

    async fn fetch(&self, number: &MediaNumber) -> Result<ScrapedMetadata, SourceError> {
        let Some(domain) = route(number) else {
            // 前缀表里没有这一族: 这一家不认这部片, 不是站点出问题.
            return Err(SourceError::NotFound {
                id: ID,
                number: number.as_str().to_owned(),
            });
        };

        let url = detail_url(domain, number);
        let html = match self.client.get_text_with(&url, &options()).await {
            Ok(html) => html,
            Err(NetError::Status { status: 404, .. }) => {
                return Err(SourceError::NotFound {
                    id: ID,
                    number: number.as_str().to_owned(),
                });
            }
            Err(error) => return Err(SourceError::transport(ID, &error)),
        };

        parse_detail(&html, number, domain)
    }
}

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: ID,
    label: "制作商官网",
    // 29 个域名没有共同入口, 因此没有单一首页可填.
    base_url: "",
    hosts: &HOSTS,
    rate: amane_net::Rate::each_second(1, 1),
    capabilities: &[Capability::Metadata],
};

/// 站点要求的请求头: 语言头决定标签文字, 缺了会拿到英文页.
fn options() -> RequestOptions {
    RequestOptions::new().header("accept-language", "ja-JP,ja;q=0.9")
}

/// 番号 → 官网域名; 前缀表里没有时返回 `None`.
fn route(number: &MediaNumber) -> Option<&'static str> {
    let prefix = series_prefix(number.as_str());
    let maker = SERIES
        .iter()
        .find(|(series, _)| *series == prefix)
        .map(|(_, maker)| *maker)?;

    MANUFACTURERS
        .iter()
        .find(|entry| entry.id == maker)
        .map(|entry| entry.domain)
}

/// 详情页地址: `https://{域名}/works/detail/{番号去分隔符小写}`.
fn detail_url(domain: &str, number: &MediaNumber) -> String {
    format!(
        "https://{domain}/works/detail/{}",
        number.as_str().to_ascii_lowercase().replace(['-', '_'], "")
    )
}

/// 番号的系列前缀: 去掉分隔符与前导数字段, 保留开头的字母段.
fn series_prefix(number: &str) -> String {
    let cleaned = number
        .trim_start_matches(|character: char| character.is_ascii_digit())
        .replace(['-', '_'], "");
    let letters: String = cleaned
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect();

    letters.to_ascii_lowercase()
}

/// 作品信息表里某一行的取值单元格.
///
/// 表格是 `div.th` + `div.td` 的兄弟对; 只认紧邻的那个 `td`, 免得串到下一行.
fn field_cell<'a>(document: &'a Html, label: &str) -> Option<ElementRef<'a>> {
    let header_selector = Selector::parse("div.th").expect("固定选择器");

    for header in document.select(&header_selector) {
        if !text_of(&header).contains(label) {
            continue;
        }
        let Some(sibling) = header.next_siblings().filter_map(ElementRef::wrap).next() else {
            continue;
        };
        if sibling
            .value()
            .has_class("td", CaseSensitivity::CaseSensitive)
        {
            return Some(sibling);
        }
    }

    None
}

/// 某一行里的链接文本.
fn field_links(cell: &ElementRef<'_>, selector: &str) -> Vec<String> {
    let selector = Selector::parse(selector).expect("固定选择器");

    cell.select(&selector)
        .map(|link| text_of(&link))
        .filter(|text| !text.is_empty())
        .collect()
}

/// 品番栏里的号码: 标签 (DVD / Blu-ray) 在子 `span` 里, 值在 `p` 的直接文本节点上.
fn identifier(document: &Html) -> Option<String> {
    let cell = field_cell(document, "品番")?;
    let paragraph = Selector::parse("p").expect("固定选择器");

    cell.select(&paragraph)
        .next()
        .map(|element| own_text_of(&element))
        .filter(|text| !text.is_empty())
}

/// 品番与请求番号的同一性: 分隔符与大小写不参与.
fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_uppercase())
        .collect()
}

fn parse_detail(
    html: &str,
    number: &MediaNumber,
    domain: &str,
) -> Result<ScrapedMetadata, SourceError> {
    let document = Html::parse_document(html);

    // 站点对不存在的作品回 200 的「找不到页面」, 与改版页要分开.
    // 标题里带 404 的是另一家的写法 (`404 | MOODYZ ...`), 因此只认段首.
    let title = crate::dom::first_text(&document, "title").unwrap_or_default();
    if title.contains("見つかりません") || title.starts_with("404") {
        return Err(SourceError::NotFound {
            id: ID,
            number: number.as_str().to_owned(),
        });
    }

    let Some(found) = identifier(&document) else {
        return Err(SourceError::unparsable(ID, "详情页没有作品信息表", html));
    };
    if normalize(&found) != normalize(number.as_str()) {
        // 页面读得出品番却不符, 说明读到的不是这部作品: 可能是地址改版把请求带到了别的页,
        // 也可能是品番栏的结构变了. 两种情况都要原文才能定位, 因此按解析失败上报,
        // 不能报未收录 —— 那会让人以为站点没有这部片.
        return Err(SourceError::unparsable(
            ID,
            format!("品番与请求不符: 页面是 {found}, 请求是 {number}"),
            html,
        ));
    }

    let actors = field_cell(&document, "女優")
        .map(|cell| field_links(&cell, r#"a[href*="/actress/detail/"]"#))
        .unwrap_or_default();
    let genres = field_cell(&document, "ジャンル")
        .map(|cell| field_links(&cell, r#"a[href*="/works/list/genre/"]"#))
        .unwrap_or_default();
    let series = field_cell(&document, "シリーズ").and_then(|cell| {
        field_links(&cell, r#"a[href*="/works/list/series/"]"#)
            .into_iter()
            .next()
    });

    let release_date = field_cell(&document, "発売日")
        .map(|cell| text_of(&cell))
        .as_deref()
        .and_then(find_date);
    let runtime_minutes = field_cell(&document, "収録時間")
        .map(|cell| text_of(&cell))
        .as_deref()
        .and_then(parse_minutes);

    // 官网只给横版封面, 没有竖版海报; 横版图以作品轮播的首图充当.
    let base = format!("https://{domain}");
    let thumb = attrs(&document, "div.p-slider img.swiper-lazy", "data-src")
        .into_iter()
        .next()
        .map(|src| absolute(&base, &src));

    Ok(ScrapedMetadata {
        title: title_of(&title),
        release_date,
        runtime_minutes,
        maker: maker_of(&title),
        series,
        actors,
        genres,
        thumb_urls: thumb.into_iter().collect(),
        ..ScrapedMetadata::default()
    })
}

/// 页面标题里「作品名 | 站点名」的前半段.
fn title_of(title: &str) -> Option<String> {
    let work = title
        .rsplit_once(" | ")
        .map_or(title, |(work, _)| work)
        .trim();

    (!work.is_empty()).then(|| work.to_owned())
}

/// 站点名里的【】是制作商, 括号里还有它的日文写法.
fn maker_of(title: &str) -> Option<String> {
    let suffix = title.rsplit_once(" | ")?.1;
    let inner = suffix.split_once('【')?.1.split_once('】')?.0;
    let name = inner.split('(').next().unwrap_or(inner).trim();

    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use amane_core::MediaNumber;

    use super::{detail_url, parse_detail, route, series_prefix};

    /// 真实页面 (S1 的 SSIS-001) 裁下来的骨架, 保留了解析器依赖的每一层.
    const DETAIL: &str = r#"
        <html><head>
          <title>作品标题 | S級女優限定のAVメーカー【S1 NO.1 STYLE (エスワン ナンバーワンスタイル)】公式サイト</title>
        </head><body>
          <section class="l-slider"><div class="p-slider -work"><div class="swiper-slide">
            <img class="swiper-lazy" data-src="https://cdn.example.invalid/cover.jpeg" alt="">
          </div></div></section>
          <div class="item"><div class="th">品番</div><div class="td">
            <div class="item -minW"><p><span class="c-tag02">DVD</span>SSIS001</p></div>
            <div class="item -minW"><p><span class="c-tag02">Blu-ray</span>9SSIS001</p></div>
          </div></div>
          <div class="item"><div class="th">発売日</div><div class="td">
            <div class="item"><a href="/works/list/date/2021-02-19">2021年2月19日</a></div>
          </div></div>
          <div class="item"><div class="th">収録時間</div><div class="td">
            <div class="item -minW"><p><span class="c-tag02">DVD</span>150分</p></div>
          </div></div>
          <div class="item"><div class="th">女優</div><div class="td">
            <div class="item"><a href="https://s1s1s1.com/actress/detail/13278">葵つかさ</a></div>
            <div class="item"><a href="https://s1s1s1.com/actress/detail/25373">乙白さやか</a></div>
          </div></div>
          <div class="item"><div class="th">シリーズ</div><div class="td"></div></div>
          <div class="item"><div class="th">ジャンル</div><div class="td">
            <div class="item"><a href="https://s1s1s1.com/works/list/genre/421">3P、4P</a></div>
            <div class="item"><a href="https://s1s1s1.com/works/list/genre/330">美少女</a></div>
          </div></div>
        </body></html>
    "#;

    /// 不存在的作品: 站点回 200 的「找不到页面」, 标题里写明.
    const MISSING: &str = r#"<html><head><title>指定されたページが見つかりません | S1</title></head><body></body></html>"#;

    /// 改版后的页面: 外壳还在, 信息表没了.
    const CHANGED: &str = r#"<html><head><title>作品标题 | S1</title></head><body><div class="item">改版了</div></body></html>"#;

    fn number(raw: &str) -> MediaNumber {
        MediaNumber::parse(raw).expect("应解析成功")
    }

    #[test]
    fn routes_by_series_prefix() {
        let cases = [
            ("SSIS-001", Some("s1s1s1.com")),
            ("JUL-001", Some("www.madonna-av.com")),
            ("IPX-001", Some("ideapocket.com")),
            ("MIDE-001", Some("moodyz.com")),
            ("ZZZ-001", None),
            ("FC2-1234567", None),
        ];

        for (raw, expected) in cases {
            assert_eq!(route(&number(raw)), expected, "输入 {raw}");
        }
    }

    #[test]
    fn strips_separators_and_leading_digits_from_the_prefix() {
        let cases = [
            ("SSIS-001", "ssis"),
            ("111111_001", ""),
            ("390JAC-079", "jac"),
            ("SSIS001", "ssis"),
        ];

        for (raw, expected) in cases {
            assert_eq!(series_prefix(raw), expected, "输入 {raw}");
        }
    }

    #[test]
    fn builds_detail_address_from_domain_and_number() {
        assert_eq!(
            detail_url("s1s1s1.com", &number("SSIS-001")),
            "https://s1s1s1.com/works/detail/ssis001"
        );
    }

    #[test]
    fn parses_the_work_table() {
        let scraped = parse_detail(DETAIL, &number("SSIS-001"), "s1s1s1.com").expect("应解析成功");

        assert_eq!(scraped.title.as_deref(), Some("作品标题"));
        assert_eq!(
            scraped.release_date.map(|date| date.to_string()),
            Some("2021-02-19".to_owned())
        );
        assert_eq!(scraped.runtime_minutes, Some(150), "标签 DVD 不参与时长");
        assert_eq!(scraped.actors, vec!["葵つかさ", "乙白さやか"]);
        assert_eq!(scraped.genres, vec!["3P、4P", "美少女"]);
        assert_eq!(scraped.maker.as_deref(), Some("S1 NO.1 STYLE"));
        assert_eq!(
            scraped.thumb_urls.first().map(String::as_str),
            Some("https://cdn.example.invalid/cover.jpeg"),
            "官网只有横版封面"
        );
        assert!(scraped.poster_urls.is_empty(), "没有竖版海报");
    }

    #[test]
    fn empty_series_cell_is_not_a_value() {
        let scraped = parse_detail(DETAIL, &number("SSIS-001"), "s1s1s1.com").expect("应解析成功");

        assert_eq!(scraped.series, None, "空单元格不是空字符串");
    }

    #[test]
    fn missing_page_is_not_found() {
        let error = parse_detail(MISSING, &number("SSIS-001"), "s1s1s1.com")
            .expect_err("找不到页面应判未收录");

        assert!(matches!(error, super::SourceError::NotFound { .. }));
    }

    #[test]
    fn changed_layout_keeps_raw() {
        let error = parse_detail(CHANGED, &number("SSIS-001"), "s1s1s1.com")
            .expect_err("没有信息表应判解析失败");

        match &error {
            super::SourceError::Parse { message, raw, .. } => {
                assert!(message.contains("信息表"), "说明应指出缺什么: {message}");
                assert_eq!(raw.content, CHANGED, "应保留完整原文");
                assert_eq!(raw.extension, "html");
            }
            other => panic!("应判解析失败, 实际 {other:?}"),
        }
    }

    #[test]
    fn page_for_another_number_is_a_parse_failure() {
        // 报未收录会让人以为站点没有这部片; 品番读得出来却不符, 要留着原文查地址与结构.
        let error = parse_detail(DETAIL, &number("ABC-999"), "s1s1s1.com")
            .expect_err("品番不符应判解析失败");

        match &error {
            super::SourceError::Parse { message, raw, .. } => {
                assert!(
                    message.contains("SSIS001"),
                    "说明应写出页面上的品番: {message}"
                );
                assert!(
                    message.contains("ABC-999"),
                    "说明应写出请求的番号: {message}"
                );
                assert_eq!(raw.content, DETAIL, "应保留完整原文");
            }
            other => panic!("应判解析失败, 实际 {other:?}"),
        }
    }

    #[test]
    fn every_series_prefix_points_at_a_known_manufacturer() {
        for (series, maker) in super::SERIES {
            assert!(
                super::MANUFACTURERS.iter().any(|entry| entry.id == *maker),
                "前缀 {series} 指向未登记的制作商 {maker}"
            );
        }
        assert_eq!(super::MANUFACTURERS.len(), 29, "29 家片商全部登记在册");
        assert_eq!(super::HOSTS.len(), 29, "限速表要覆盖每一家域名");
    }
}
