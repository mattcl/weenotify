use message::Message;
use slog;
use std::collections::HashSet;

pub struct Filter {
    ignored_tags: HashSet<String>,
    ignored_senders: HashSet<String>,
    log: slog::Logger,
}

impl Filter {
    pub fn should_filter(&self, message: &Message) -> bool {
        if let Some(sender) = message.sender() {
            if self.ignored_senders.contains(&sender) {
                return true;
            }
        }
        for tag in &message.tags {
            if self.ignored_tags.contains(tag) {
                return true;
            }
        }
        false
    }
}
