//! # twilight-bucket
//! a [twilight](https://docs.rs/twilight) utility crate to limit users' usage
//!
//! all the functionality of this crate is under [`Bucket`], see its
//! documentation for usage info
//!
//! this crate can be used with any library, but it shares twilight's non-goals,
//! such as trying to be more verbose and less opinionated
//! and [serenity already has a bucket implementation][serenity bucket]
//!
//! [serenity bucket]: https://docs.rs/serenity/latest/serenity/framework/standard/buckets
//! # example
//! ```
//! use std::{num::NonZeroU64, time::Duration};
//! use twilight_bucket::{Bucket, Limit};
//!
//! #[tokio::main]
//! async fn main() {
//!     // a user can use it once every 10 seconds
//!     let my_command_user_bucket =
//!         Bucket::new(Limit::new(Duration::from_secs(10), 1.try_into()?));
//!     // it can be used up to 5 times every 30 seconds in one channel
//!     let my_command_channel_bucket =
//!         Bucket::new(Limit::new(Duration::from_secs(30), 5.try_into()?));
//!     run_my_command(
//!         my_command_user_bucket,
//!         my_command_channel_bucket,
//!         12345.try_into()?,
//!         123.try_into()?,
//!     )
//!     .await;
//! }
//!
//! async fn run_my_command(
//!     user_bucket: Bucket,
//!     channel_bucket: Bucket,
//!     user_id: NonZeroU64,
//!     channel_id: NonZeroU64,
//! ) -> String {
//!     if let Some(channel_limit_duration) = channel_bucket.limit_duration(channel_id) {
//!         return format!(
//!             "this was used too much in this channel, please wait {} seconds",
//!             channel_limit_duration.as_secs()
//!         );
//!     }
//!     if let Some(user_limit_duration) = user_bucket.limit_duration(user_id) {
//!         if Duration::from_secs(5) > user_limit_duration {
//!             tokio::time::sleep(user_limit_duration).await;
//!         } else {
//!             return format!(
//!                 "you've been using this too much, please wait {} seconds",
//!                 user_limit_duration.as_secs()
//!             );
//!         }
//!     }
//!     user_bucket.register(user_id);
//!     channel_bucket.register(channel_id);
//!     "ran your command".to_owned()
//! }
//! ```

#![warn(clippy::cargo, clippy::nursery, clippy::pedantic, clippy::restriction)]
#![allow(
    clippy::blanket_clippy_restriction_lints,
    clippy::missing_inline_in_public_items,
    clippy::implicit_return,
    clippy::shadow_same,
    clippy::separated_literal_suffix
)]

use std::{
    num::{NonZeroU64, NonZeroUsize},
    time::{Duration, Instant},
};

use dashmap::DashMap;

/// information about how often something is able to be used
///
/// # examples
/// something can be used every 3 seconds
/// ```
/// twilight_bucket::Limit::new(std::time::Duration::from_secs(3), 1.try_into()?);
/// ```
/// something can be used 10 times in 1 minute, so the limit resets every minute
/// ```
/// twilight_bucket::Limit::new(std::time::Duration::from_secs(60), 10.try_into()?);
/// ```
#[must_use]
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct Limit {
    /// how often something can be done [`Limit::count`] times
    duration: Duration,
    /// how many times something can be done in the [`Limit::duration`] period
    count: NonZeroUsize,
}

impl Limit {
    /// create a new [`Limit`]
    pub const fn new(duration: Duration, count: NonZeroUsize) -> Self {
        Self { duration, count }
    }
}

/// usage information about an ID
#[must_use]
#[derive(Copy, Clone)]
struct Usage {
    /// the last time it was used
    time: Instant,
    /// how many times it was used
    count: NonZeroUsize,
}

impl Usage {
    /// make a usage with now as `time` and 1 as `count`
    #[allow(clippy::unwrap_used)]
    fn new() -> Self {
        Self {
            time: Instant::now(),
            count: 1.try_into().unwrap(),
        }
    }
}

/// this is the main struct to do everything you need
///
/// # thread-safety
/// you should wrap this in [`Arc`](std::sync::Arc)
///
/// # global or task-based
/// essentially buckets just store usages and limits, meaning you can create a
/// different bucket for each kind of limit: each of your commands, separate
/// buckets for channel and user usage if you want to have different limits for
/// each etc.
///
/// # usage
/// register usages using the [`Bucket::register`] method **after** getting the
/// limit with [`Bucket::limit_duration`]
///
/// `ID`s use [`NonZeroU64`](std::num::NonZeroU64) to be compatible with any
/// kind of ID: users, guilds, even your custom IDs
#[must_use]
#[derive(Clone)]
pub struct Bucket {
    /// the limit for this bucket
    limit: Limit,
    /// usage information for IDs
    usages: DashMap<NonZeroU64, Usage>,
}

impl Bucket {
    /// create a new [`Bucket`] with the given limit
    pub fn new(limit: Limit) -> Self {
        Self {
            limit,
            usages: DashMap::new(),
        }
    }

    /// register a usage, you should call this every time something you want to
    /// limit is done **after** waiting for the limit
    ///
    /// # Panics
    /// when the usage count is over `NonZeroUsize`
    #[allow(clippy::unwrap_used, clippy::integer_arithmetic)]
    pub fn register(&self, id: NonZeroU64) {
        match self.usages.get_mut(&id) {
            Some(mut usage) => {
                let now = Instant::now();
                usage.count = if now - usage.time > self.limit.duration {
                    1.try_into().unwrap()
                } else {
                    (usage.count.get() + 1).try_into().unwrap()
                };
                usage.time = now;
            }
            None => {
                self.usages.insert(id, Usage::new());
            }
        }
    }

    /// get the duration to wait until the next usage by `id`, returns `None`
    /// if the ID isn't limited, you should call this **before** registering a
    /// usage
    #[must_use]
    pub fn limit_duration(&self, id: NonZeroU64) -> Option<Duration> {
        let usage = self.usages.get(&id)?;
        let elapsed = Instant::now() - usage.time;
        (usage.count >= self.limit.count && self.limit.duration > elapsed)
            .then(|| self.limit.duration - elapsed)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::sleep;

    use crate::{Bucket, Limit};

    #[allow(clippy::unwrap_used)]
    #[tokio::test]
    async fn limit_count_1() {
        let bucket = Bucket::new(Limit::new(Duration::from_secs(2), 1.try_into().unwrap()));
        let id = 123.try_into().unwrap();

        assert!(bucket.limit_duration(id).is_none());

        bucket.register(id);
        assert!(
            bucket.limit_duration(id).unwrap()
                > bucket.limit.duration - Duration::from_secs_f32(0.1)
        );
        sleep(bucket.limit.duration).await;
        assert!(bucket.limit_duration(id).is_none());
    }

    #[allow(clippy::unwrap_used)]
    #[tokio::test]
    async fn limit_count_5() {
        let bucket = Bucket::new(Limit::new(Duration::from_secs(5), 5.try_into().unwrap()));
        let id = 123.try_into().unwrap();

        for _ in 0_u8..5 {
            assert!(bucket.limit_duration(id).is_none());
            bucket.register(id);
        }

        assert!(
            bucket.limit_duration(id).unwrap()
                > bucket.limit.duration - Duration::from_secs_f32(0.1)
        );
        sleep(bucket.limit.duration).await;
        assert!(bucket.limit_duration(id).is_none());
    }
}
