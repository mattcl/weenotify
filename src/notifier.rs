use notify_rust::Notification;
use slog;

pub struct Notifier {
    pub duration: i32,
    pub log: slog::Logger,
}

impl Notifier {
    pub fn new(log: &slog::Logger, duration: i32) -> Self {
        Notifier {
            duration: duration,
            log: log.new(o!()),
        }
    }

    pub fn show(&self, summary: &str, body: &str) {
        match Notification::new()
                  .summary(summary)
                  .body(body)
                  .timeout(self.duration)
                  .show() {
            Ok(_) => (),
            Err(_) => {
                error!(self.log, "error displaying notification"; "summary" => summary.clone(), "body" => body.clone());
            }
        }
    }
}
