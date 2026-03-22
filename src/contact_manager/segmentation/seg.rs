#[cfg(feature = "first_depleted")]
use crate::types::Volume;
use crate::{
    bundle::Bundle,
    contact::ContactInfo,
    contact_manager::{
        ContactManager, ContactManagerTxData,
        segmentation::{BaseSegmentationManager, Segment},
    },
    parsing::{DispatchParser, Lexer, Parser, ParsingState},
    types::{DataRate, Date, Duration},
};

/// Manages contact segments, where each segment may have a distinct data rate and delay.
///
/// The `SegmentationManager` uses different segments to manage free intervals, rate intervals, and delay intervals,
/// which are applied in contact scheduling and transmission simulation.
#[cfg_attr(feature = "debug", derive(Debug))]
pub struct SegmentationManager {
    /// A list of segments representing free intervals available for transmission.
    free_intervals: Vec<Segment<()>>,
    /// A list of segments representing different data rates during contact intervals.
    rate_intervals: Vec<Segment<DataRate>>,
    /// A list of segments representing delay times associated with different intervals.
    delay_intervals: Vec<Segment<Duration>>,
    #[cfg(feature = "first_depleted")]
    /// The total volume at initialization.
    original_volume: Volume,
}

impl SegmentationManager {
    /// Creates a new [`SegmentationManager`] from the provided rate and delay intervals.
    ///
    /// This constructor initializes the manager with:
    /// - An empty set of `free_intervals`
    /// - The given `rate_intervals`, which define data rates over contact segments
    /// - The given `delay_intervals`, which define propagation or processing delays
    ///
    /// # Arguments
    ///
    /// * `rate_intervals` - Segments describing data rates over time.
    /// * `delay_intervals` - Segments describing delay durations over time.
    ///
    /// # Feature Flags
    ///
    /// When the `first_depleted` feature is enabled, the `original_volume`
    /// field is initialized to `0.0`.
    ///
    /// # Returns
    ///
    /// A fully initialized [`SegmentationManager`].
    pub fn new(
        rate_intervals: Vec<Segment<DataRate>>,
        delay_intervals: Vec<Segment<Duration>>,
    ) -> Self {
        let free_intervals = Vec::new();

        Self {
            free_intervals,
            rate_intervals,
            delay_intervals,
            #[cfg(feature = "first_depleted")]
            original_volume: 0.0,
        }
    }
}

impl BaseSegmentationManager for SegmentationManager {
    /// Delegates construction to [`SegmentationManager::new`].
    fn new(
        rate_intervals: Vec<Segment<DataRate>>,
        delay_intervals: Vec<Segment<Duration>>,
    ) -> Self {
        Self::new(rate_intervals, delay_intervals)
    }
}

/// Implements the `ContactManager` trait for `SegmentationManager`, providing methods for simulating and scheduling transmissions.
impl ContactManager for SegmentationManager {
    /// Simulates the transmission of a bundle based on the contact data and available free intervals.
    ///
    /// # Arguments
    ///
    /// * `_contact_data` - Reference to the contact information (unused in this implementation).
    /// * `at_time` - The current time for scheduling purposes.
    /// * `bundle` - The bundle to be transmitted.
    ///
    /// # Returns
    ///
    /// Optionally returns `ContactManagerTxData` with transmission start and end times, or `None` if the bundle can't be transmitted.
    fn dry_run_tx(
        &self,
        _contact_data: &ContactInfo,
        at_time: Date,
        bundle: &Bundle,
    ) -> Option<ContactManagerTxData> {
        let mut tx_start: Date;

        for free_seg in &self.free_intervals {
            if free_seg.end < at_time {
                continue;
            }
            tx_start = Date::max(free_seg.start, at_time);
            let Some(tx_end) =
                super::get_tx_end(&self.rate_intervals, tx_start, bundle.size, free_seg.end)
            else {
                continue;
            };

            let (d_start, d_end) = super::get_delays(tx_start, tx_end, &self.delay_intervals);
            return Some(ContactManagerTxData {
                tx_start,
                tx_end,
                expiration: free_seg.end,
                rx_start: tx_start + d_start,
                rx_end: tx_end + d_end,
            });
        }
        None
    }

    /// Schedule the transmission of a bundle by splitting the available free intervals accordingly.
    ///
    /// This method shall be called after a dry run ! Implementations might not ensure a clean behavior otherwise.
    ///
    /// # Arguments
    ///
    /// * `_contact_data` - Reference to the contact information (unused in this implementation).
    /// * `at_time` - The current time for scheduling purposes.
    /// * `bundle` - The bundle to be transmitted.
    ///
    /// # Returns
    ///
    /// Optionally returns `ContactManagerTxData` with transmission start and end times, or `None` if the bundle can't be transmitted.
    fn schedule_tx(
        &mut self,
        _contact_data: &ContactInfo,
        at_time: Date,
        bundle: &Bundle,
    ) -> Option<ContactManagerTxData> {
        let mut tx_start = 0.0;
        let mut index = 0;
        let mut tx_end = 0.0;

        for free_seg in &self.free_intervals {
            if free_seg.end < at_time {
                continue;
            }
            tx_start = Date::max(free_seg.start, at_time);
            if let Some(tx_end_res) =
                super::get_tx_end(&self.rate_intervals, tx_start, bundle.size, free_seg.end)
            {
                tx_end = tx_end_res;
                break;
            }
            index += 1;
        }

        let interval = &mut self.free_intervals[index];
        let expiration = interval.end;
        let (d_start, d_end) = super::get_delays(tx_start, tx_end, &self.delay_intervals);

        if interval.start != tx_start {
            interval.end = tx_start;
            self.free_intervals.insert(
                index + 1,
                Segment {
                    start: tx_end,
                    end: expiration,
                    val: (),
                },
            )
        } else {
            interval.start = tx_end;
        }

        Some(ContactManagerTxData {
            tx_start,
            tx_end,
            expiration,
            rx_start: tx_start + d_start,
            rx_end: tx_end + d_end,
        })
    }

    /// Initializes the segmentation manager by checking that rate and delay intervals have no gaps.
    ///
    /// # Arguments
    ///
    /// * `contact_data` - Reference to the contact information.
    ///
    /// # Returns
    ///
    /// Returns `true` if initialization is successful, or `false` if there are gaps in the intervals.
    fn try_init(&mut self, contact_data: &ContactInfo) -> bool {
        super::try_init(
            &self.rate_intervals,
            &self.delay_intervals,
            &mut self.free_intervals,
            (),
            #[cfg(feature = "first_depleted")]
            &mut self.original_volume,
            contact_data,
        )
    }

    /// For first depleted compatibility
    ///
    /// # Returns
    ///
    /// Returns the maximum volume the contact had at initialization.
    #[cfg(feature = "first_depleted")]
    fn get_original_volume(&self) -> Volume {
        self.original_volume
    }
}

/// Implements the DispatchParser to allow dynamic parsing.
impl DispatchParser<SegmentationManager> for SegmentationManager {}

/// Implements the `Parser` trait for `SegmentationManager`, allowing the manager to be parsed from a lexer.
impl Parser<SegmentationManager> for SegmentationManager {
    /// Parses a `SegmentationManager` from the lexer, extracting the rate and delay intervals.
    ///
    /// # Arguments
    ///
    /// * `lexer` - The lexer used for parsing tokens.
    ///
    /// # Returns
    ///
    /// Returns a `ParsingState` indicating whether parsing was successful (`Finished`) or encountered an error (`Error`).
    fn parse(lexer: &mut dyn Lexer) -> ParsingState<SegmentationManager> {
        super::parse::<SegmentationManager>(lexer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contact_manager::segmentation::BaseSegmentationManager;

    #[test]
    fn test_new_manager() {
        // We create simple segments for rate and delay.
        // A segment represents a time interval with a value.
        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 2.0,
        }];

        let delay_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        // Create the segmentation manager
        let manager = SegmentationManager::new(rate_intervals, delay_intervals);

        // When the manager is created, free_intervals should be empty
        assert!(manager.free_intervals.is_empty());

        // Check that the rate intervals were stored correctly
        assert_eq!(manager.rate_intervals.len(), 1);
        assert_eq!(manager.rate_intervals[0].start, 0.0);
        assert_eq!(manager.rate_intervals[0].end, 10.0);
        assert_eq!(manager.rate_intervals[0].val, 2.0);

        // Check that the delay intervals were stored correctly
        assert_eq!(manager.delay_intervals.len(), 1);
        assert_eq!(manager.delay_intervals[0].start, 0.0);
        assert_eq!(manager.delay_intervals[0].end, 10.0);
        assert_eq!(manager.delay_intervals[0].val, 1.0);
    }

    #[test]
    fn test_manager_stores_multiple_intervals() {
        // This test checks that the manager correctly stores multiple segments.

        let rate_intervals = vec![
            Segment { start: 0.0, end: 5.0, val: 1.0 },
            Segment { start: 5.0, end: 10.0, val: 2.0 },
        ];

        let delay_intervals = vec![
            Segment { start: 0.0, end: 5.0, val: 0.5 },
            Segment { start: 5.0, end: 10.0, val: 1.0 },
        ];

        let manager = SegmentationManager::new(rate_intervals, delay_intervals);

        // Check that both segments were stored
        assert_eq!(manager.rate_intervals.len(), 2);
        assert_eq!(manager.delay_intervals.len(), 2);
    }

    #[test]
    fn test_manager_keeps_segment_values() {
        // This test verifies that the values inside segments are not modified
        // when the manager is created.

        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 20.0,
            val: 5.0,
        }];

        let delay_intervals = vec![Segment {
            start: 0.0,
            end: 20.0,
            val: 3.0,
        }];

        let manager = SegmentationManager::new(rate_intervals, delay_intervals);

        // Check stored values
        assert_eq!(manager.rate_intervals[0].val, 5.0);
        assert_eq!(manager.delay_intervals[0].val, 3.0);
    }

    #[test]
    fn test_manager_initial_state() {
        // This test checks the initial state of the manager after creation.

        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let delay_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let manager = SegmentationManager::new(rate_intervals, delay_intervals);

        // The manager should start with no free intervals
        assert!(manager.free_intervals.is_empty());
    }

    #[test]
    fn test_new_manager_from_trait() {
        // Same idea as the previous test, but using the trait constructor
        // instead of calling SegmentationManager::new directly.

        let rate_intervals = vec![Segment {
            start: 5.0,
            end: 15.0,
            val: 4.0,
        }];

        let delay_intervals = vec![Segment {
            start: 5.0,
            end: 15.0,
            val: 2.0,
        }];

        // Create the manager through the BaseSegmentationManager trait
        let manager =
            <SegmentationManager as BaseSegmentationManager>::new(
                rate_intervals,
                delay_intervals,
            );

        // The manager should contain the intervals we gave
        assert!(manager.free_intervals.is_empty());

        assert_eq!(manager.rate_intervals.len(), 1);
        assert_eq!(manager.rate_intervals[0].start, 5.0);
        assert_eq!(manager.rate_intervals[0].end, 15.0);
        assert_eq!(manager.rate_intervals[0].val, 4.0);

        assert_eq!(manager.delay_intervals.len(), 1);
        assert_eq!(manager.delay_intervals[0].start, 5.0);
        assert_eq!(manager.delay_intervals[0].end, 15.0);
        assert_eq!(manager.delay_intervals[0].val, 2.0);
    }
    #[test]
    fn test_dry_run_returns_none_when_not_initialized() {
        // The manager starts with no free interval.
        // So dry_run_tx should return None.

        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 2.0,
        }];

        let delay_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let manager = SegmentationManager::new(rate_intervals, delay_intervals);

        //ContactInfo(tx_node, rx_node, start, end)
        let contact = ContactInfo::new(1, 2, 0.0, 10.0);

        let bundle = Bundle {
            source: 1,
            destinations: vec![2],
            priority: 0,
            size: 4.0,
            expiration: 100.0,
        };

        let result = manager.dry_run_tx(&contact, 0.0, &bundle);

        assert!(result.is_none());
    }

    #[test]
    fn test_dry_run_returns_some_after_init() {
        // After try_init, the manager has one free interval.
        // Here the bundle can be transmitted, so dry_run_tx should return Some.

        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 2.0,
        }];

        let delay_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let mut manager = SegmentationManager::new(rate_intervals, delay_intervals);

        //ContactInfo(tx_node, rx_node, start, end)
        let contact = ContactInfo::new(1, 2, 0.0, 10.0);
        assert!(manager.try_init(&contact));

        let bundle = Bundle {
            source: 1,
            destinations: vec![2],
            priority: 0,
            size: 4.0,
            expiration: 100.0,
        };

        let result = manager.dry_run_tx(&contact, 0.0, &bundle);

        assert!(result.is_some());
        //Option<> -> Some(valeur) ou None
    }

    #[test]
    fn test_dry_run_uses_at_time_as_start_when_inside_contact() {
        // If at_time is inside the free interval,
        // the transmission should start at at_time.

        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 2.0,
        }];

        let delay_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let mut manager = SegmentationManager::new(rate_intervals, delay_intervals);

        let contact = ContactInfo::new(1, 2, 0.0, 10.0);
        assert!(manager.try_init(&contact));

        let bundle = Bundle {
            source: 1,
            destinations: vec![2],
            priority: 0,
            size: 4.0,
            expiration: 100.0,
        };

        let result = manager.dry_run_tx(&contact, 3.0, &bundle).unwrap();
        //at_time = 3.0

        // size = 4, rate = 2, so transmission duration = 4/2 = 2
        //max(free_seg.start, at_time)
        assert_eq!(result.tx_start, 3.0);
    }

    #[test]
    fn test_dry_run_uses_contact_start_when_at_time_is_before_contact() {
        // If at_time is before the contact,
        // the transmission should start at the beginning of the free interval.

        let rate_intervals = vec![Segment {
            start: 5.0,
            end: 15.0,
            val: 2.0,
        }];

        let delay_intervals = vec![Segment {
            start: 5.0,
            end: 15.0,
            val: 1.0,
        }];

        let mut manager = SegmentationManager::new(rate_intervals, delay_intervals);

        let contact = ContactInfo::new(1, 2, 5.0, 15.0);
        assert!(manager.try_init(&contact));

        let bundle = Bundle {
            source: 1,
            destinations: vec![2],
            priority: 0,
            size: 4.0,
            expiration: 100.0,
        };

        let result = manager.dry_run_tx(&contact, 0.0, &bundle).unwrap();

        assert_eq!(result.tx_start, 5.0);
    }

    #[test]
    fn test_dry_run_returns_none_when_bundle_is_too_large() {
        // The bundle is too large to finish before the end of the free interval.
        // So dry_run_tx should return None.

        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let delay_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let mut manager = SegmentationManager::new(rate_intervals, delay_intervals);

        let contact = ContactInfo::new(1, 2, 0.0, 10.0);
        assert!(manager.try_init(&contact));

        let bundle = Bundle {
            source: 1,
            destinations: vec![2],
            priority: 0,
            size: 20.0,
            expiration: 100.0,
        };

        let result = manager.dry_run_tx(&contact, 0.0, &bundle);

        assert!(result.is_none());
    }

    #[test]
    fn test_dry_run_uses_the_correct_values() {
        // This test checks that the values returned by dry_run_tx are correct 
        // based on the rate and delay intervals.

        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let delay_intervals = vec![
            Segment {
                start: 0.0,
                end: 4.0,
                val: 1.0,
            },
            Segment {
                start: 4.0,
                end: 10.0,
                val: 3.0,
            },
        ];

        let mut manager = SegmentationManager::new(rate_intervals, delay_intervals);

        let contact = ContactInfo::new(1, 2, 0.0, 10.0);
        assert!(manager.try_init(&contact));

        let bundle = Bundle {
            source: 1,
            destinations: vec![2],
            priority: 0,
            size: 5.0,
            expiration: 100.0,
        };

        let result = manager.dry_run_tx(&contact, 0.0, &bundle).unwrap();

        // size = 5, rate = 1 -> tx_end = 5
        // tx_end = 5 is in the second delay interval, so delay = 3
        //max(free_seg.start, at_time)
        assert_eq!(result.tx_start, 0.0);
        //bundle.size = 5, rate_intervals[0].val = 1, transmission duration = 5, tx_end = 0+5 = 5
        assert_eq!(result.tx_end, 5.0);
        //delay_intervals[1].val
        assert_eq!(result.delay, 3.0);
        //contact.end = 10.0
        assert_eq!(result.expiration, 10.0);
        //tx_end + delay
        assert_eq!(result.arrival, 8.0);
    }

    //free_intervals : seg works with free intervals and reserves some place in them.
    //No priority in this manager, so the first free interval that can fit the bundle is chosen.
}