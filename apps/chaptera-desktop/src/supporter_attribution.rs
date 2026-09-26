use crate::supporter::{MarketProfile, SupporterAction, ValueReceipt, ValueReceiptKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportMarket {
    Us,
    Uk,
    Ru,
}

impl SupportMarket {
    fn from_profile(profile: MarketProfile) -> Option<Self> {
        match profile {
            MarketProfile::Us => Some(Self::Us),
            MarketProfile::Uk => Some(Self::Uk),
            MarketProfile::Ru => Some(Self::Ru),
            MarketProfile::NeutralEnglish => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Us => "us",
            Self::Uk => "uk",
            Self::Ru => "ru",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportCopyVariant {
    ControlV1,
}

impl SupportCopyVariant {
    fn as_str(self) -> &'static str {
        match self {
            Self::ControlV1 => "control-v1",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportValueClass {
    Reading,
    Search,
    Copy,
}

impl SupportValueClass {
    fn from_receipt(receipt: ValueReceipt) -> Self {
        match receipt.kind {
            ValueReceiptKind::Reading { .. } => Self::Reading,
            ValueReceiptKind::SearchMatches { .. } => Self::Search,
            ValueReceiptKind::TextCopied => Self::Copy,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Reading => "reading",
            Self::Search => "search",
            Self::Copy => "copy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SupportClickAttribution {
    market: SupportMarket,
    variant: SupportCopyVariant,
    value_class: SupportValueClass,
    impression_index: u8,
}

impl SupportClickAttribution {
    pub(crate) fn for_action(
        action: SupporterAction,
        profile: MarketProfile,
        receipt: ValueReceipt,
        impression_index: u8,
    ) -> Option<Self> {
        if action != SupporterAction::Support {
            return None;
        }
        Self::baseline(profile, receipt, impression_index)
    }

    fn baseline(
        profile: MarketProfile,
        receipt: ValueReceipt,
        impression_index: u8,
    ) -> Option<Self> {
        if !(1..=2).contains(&impression_index) {
            return None;
        }

        Some(Self {
            market: SupportMarket::from_profile(profile)?,
            variant: SupportCopyVariant::ControlV1,
            value_class: SupportValueClass::from_receipt(receipt),
            impression_index,
        })
    }

    pub(crate) fn query_string(self) -> String {
        format!(
            "m={}&v={}&ve={}&i={}",
            self.market.as_str(),
            self.variant.as_str(),
            self.value_class.as_str(),
            self.impression_index
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(kind: ValueReceiptKind) -> ValueReceipt {
        ValueReceipt {
            page_count: 14,
            kind,
        }
    }

    #[test]
    fn explicit_support_click_builds_the_closed_tuple() {
        let reading = receipt(ValueReceiptKind::Reading {
            text_searchable: true,
        });
        assert_eq!(
            SupportClickAttribution::for_action(
                SupporterAction::Support,
                MarketProfile::Us,
                reading,
                1
            )
            .expect("US support attribution")
            .query_string(),
            "m=us&v=control-v1&ve=reading&i=1"
        );
        assert_eq!(
            SupportClickAttribution::for_action(
                SupporterAction::Support,
                MarketProfile::Uk,
                reading,
                2
            )
            .expect("UK support attribution")
            .query_string(),
            "m=uk&v=control-v1&ve=reading&i=2"
        );
        assert_eq!(
            SupportClickAttribution::for_action(
                SupporterAction::Support,
                MarketProfile::Ru,
                reading,
                1
            )
            .expect("RU support attribution")
            .query_string(),
            "m=ru&v=control-v1&ve=reading&i=1"
        );
    }

    #[test]
    fn share_report_and_archive_never_inherit_support_attribution() {
        let reading = receipt(ValueReceiptKind::Reading {
            text_searchable: true,
        });
        for action in [
            SupporterAction::Share,
            SupporterAction::Report,
            SupporterAction::ArchiveHelp,
            SupporterAction::Later,
            SupporterAction::AlreadySupported,
        ] {
            assert_eq!(
                SupportClickAttribution::for_action(action, MarketProfile::Us, reading, 1),
                None
            );
        }
    }

    #[test]
    fn value_class_discards_document_metrics() {
        let attribution = SupportClickAttribution::for_action(
            SupporterAction::Support,
            MarketProfile::Ru,
            ValueReceipt {
                page_count: 123_456,
                kind: ValueReceiptKind::SearchMatches {
                    match_count: 654_321,
                },
            },
            2,
        )
        .expect("bounded attribution");

        let query = attribution.query_string();
        assert_eq!(query, "m=ru&v=control-v1&ve=search&i=2");
        for forbidden in [
            "123456", "654321", "path", "hash", "text", "account", "machine", "install",
        ] {
            assert!(!query.contains(forbidden), "query leaked {forbidden}");
        }
    }

    #[test]
    fn neutral_market_and_out_of_contract_impressions_fail_closed() {
        let reading = receipt(ValueReceiptKind::Reading {
            text_searchable: false,
        });

        assert_eq!(
            SupportClickAttribution::for_action(
                SupporterAction::Support,
                MarketProfile::NeutralEnglish,
                reading,
                1
            ),
            None
        );
        assert_eq!(
            SupportClickAttribution::for_action(
                SupporterAction::Support,
                MarketProfile::Us,
                reading,
                0
            ),
            None
        );
        assert_eq!(
            SupportClickAttribution::for_action(
                SupporterAction::Support,
                MarketProfile::Us,
                reading,
                3
            ),
            None
        );
    }

    #[test]
    fn no_route_builder_performs_network_io() {
        let source = include_str!("supporter_attribution.rs");
        assert!(!source.contains(concat!("req", "west")));
        assert!(!source.contains(concat!("u", "req")));
        assert!(!source.contains(concat!("Tcp", "Stream")));
        assert!(!source.contains(concat!("open", "_url")));
    }
}
