use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use esp_idf_svc::sntp::{EspSntp, SyncStatus::Completed};
use std::{thread::sleep, time::Duration};

pub fn sync_time() -> String {
    log::info!("SNTP sync time");
    show_now();
    let ntp_client = EspSntp::new_default().unwrap();
    loop {
        let status = ntp_client.get_sync_status();
        log::debug!("sntp sync status {:?}", status);
        if status == Completed {
            break;
        }
        sleep(Duration::from_secs(1));
    }
    log::info!("SNTP synchronized!");
    show_now()
}

fn show_now() -> String {
    let cst = FixedOffset::east_opt(8 * 3600).unwrap(); // 安全的偏移量创建

    let utc_now = Utc::now();
    let local_now: DateTime<FixedOffset> = cst.from_utc_datetime(&utc_now.naive_utc());
    let now_str = local_now.format("%Y-%m-%dT%H:%M:%S%:z").to_string();

    log::info!("now time: {}", now_str);

    now_str.to_string()
}
