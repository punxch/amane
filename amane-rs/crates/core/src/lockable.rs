use std::fmt;

/// 可锁定的字段.
///
/// 锁定表示「这个值由人决定」: 重新刮削时跳过该字段, 字段来源保持原样.
/// 演员名单不在其中 —— 它是整体替换的列表, 没有逐项修订的语义.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LockableField {
    Title,
    Plot,
    ReleaseDate,
    RuntimeMinutes,
    Maker,
    Label,
    Series,
    PosterUrl,
}

impl LockableField {
    /// 全部取值, 顺序即详情页上的展示顺序.
    pub const ALL: [Self; 8] = [
        Self::Title,
        Self::Plot,
        Self::ReleaseDate,
        Self::RuntimeMinutes,
        Self::Maker,
        Self::Label,
        Self::Series,
        Self::PosterUrl,
    ];

    /// 存储与传输使用的标识; 也是 `field_sources` 的键.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Plot => "plot",
            Self::ReleaseDate => "release_date",
            Self::RuntimeMinutes => "runtime_minutes",
            Self::Maker => "maker",
            Self::Label => "label",
            Self::Series => "series",
            Self::PosterUrl => "poster_url",
        }
    }

    /// 面向用户的名称.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Title => "标题",
            Self::Plot => "简介",
            Self::ReleaseDate => "上映日期",
            Self::RuntimeMinutes => "时长",
            Self::Maker => "制作商",
            Self::Label => "厂牌",
            Self::Series => "系列",
            Self::PosterUrl => "海报",
        }
    }

    /// 由外部字符串还原; 未知取值返回 `None`, 由调用方决定如何报错.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
    }
}

impl fmt::Display for LockableField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::LockableField;

    #[test]
    fn every_variant_round_trips_through_parse() {
        for field in LockableField::ALL {
            assert_eq!(LockableField::parse(field.as_str()), Some(field));
            assert!(!field.label().is_empty(), "每个字段都要有展示名称");
        }

        for unknown in ["", "actors", "Title", "title "] {
            assert_eq!(LockableField::parse(unknown), None, "输入 {unknown:?}");
        }
    }
}
