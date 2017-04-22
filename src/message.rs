use regex::Regex;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    msg_type: String,
    highlight: bool,
    message: String,
    away: bool,
    channel: String,
    server: String,
    date: String,
    tags: Vec<String>,
}

impl Message {
    pub fn sender(&self) -> Option<String> {
        lazy_static! {
            static ref SENDER_MATCHER: Regex = Regex::new(r"^nick_(.*)$").unwrap();
        }

        for tag in &self.tags {
            if let Some(caps) = SENDER_MATCHER.captures(tag) {
                return Some(caps.get(1).unwrap().as_str().to_owned())
            }
        }
        None
    }

    pub fn has_tag(&self, desired: &str) -> bool {
        for tag in &self.tags {
            if desired == tag {
                return true
            }
        }

        false
    }
}
