use regex::Regex;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub msg_type: String,
    pub highlight: bool,
    pub message: String,
    pub away: bool,
    pub channel: String,
    pub server: String,
    pub date: String,
    pub tags: Vec<String>,
}

impl Message {
    pub fn sender(&self) -> Option<String> {
        lazy_static! {
            static ref SENDER_MATCHER: Regex = Regex::new(r"^nick_(.*)$").unwrap();
        }

        for tag in &self.tags {
            if let Some(caps) = SENDER_MATCHER.captures(tag) {
                return Some(caps.get(1).unwrap().as_str().to_owned());
            }
        }
        None
    }

    pub fn summary(&self) -> String {
        if self.msg_type == "private" {
            return self.sender().unwrap_or("unknown".to_string());
        }
        self.channel.clone()
    }

    pub fn body(&self) -> String {
        self.message.clone()
    }
}
