use message::Message;
use slog;
use std::collections::HashSet;
use std::iter::FromIterator;
use config::Value;

pub struct Filter {
    ignored_tags: HashSet<String>,
    ignored_senders: HashSet<String>,
    log: slog::Logger,
}

impl Filter {
    pub fn new(log: &slog::Logger,
               ignored_senders: &Vec<Value>,
               ignored_tags: &Vec<Value>)
               -> Self {
        let ignored_senders = HashSet::from_iter(ignored_senders
                                                     .iter()
                                                     .map(|x| x.clone().into_str())
                                                     .filter(|x| x.is_some())
                                                     .map(|x| x.unwrap()));
        let ignored_tags = HashSet::from_iter(ignored_tags
                                                  .iter()
                                                  .map(|x| x.clone().into_str())
                                                  .filter(|x| x.is_some())
                                                  .map(|x| x.unwrap()));

        Filter {
            ignored_senders: ignored_senders,
            ignored_tags: ignored_tags,
            log: log.new(o!()),
        }
    }

    pub fn should_filter(&self, message: &Message) -> bool {
        if let Some(sender) = message.sender() {
            if self.ignored_senders.contains(&sender) {
                debug!(self.log, "ignoring message from sender"; "sender" => sender.clone());
                return true;
            }
        }
        for tag in &message.tags {
            if self.ignored_tags.contains(tag) {
                debug!(self.log, "ignoring message with tag"; "tag" => tag.clone());
                return true;
            }
        }
        false
    }
}
