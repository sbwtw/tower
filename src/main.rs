
#[macro_use]
extern crate hyper;
extern crate regex;

mod database;

use std::io::*;
use std::collections::HashMap;

use database::SqliteCookie;

use hyper::client::*;
use hyper::header::*;

use regex::Regex;

header! { (XCSRFToken, "X-CSRF-Token") => [String] }

struct App {
    tid: String,
    uid: String,
    conn_guid: String,
    headers: Headers,
    member_list: HashMap<String, String>,
}

impl App {
    pub fn new() -> App {
        App {
            tid: String::new(),
            uid: String::new(),
            conn_guid: String::new(),
            headers: Headers::new(),
            member_list: HashMap::with_capacity(200),
        }
    }

    pub fn load_sqlite<T: AsRef<str>>(&mut self, file: T) -> bool {
        let mut sc = SqliteCookie::new(file);
        let _ = sc.read_data();

        // set headers
        self.tid = sc.team_id().clone();

        let session = sc.session_id();
        let token = sc.token();

        let header_cookie = Cookie(vec![
                CookiePair::new("remember_team_guid".to_owned(), self.tid.clone()),
                CookiePair::new("remember_token".to_owned(), token.clone()),
                CookiePair::new("_tower2_session".to_owned(), session.clone()),
            ]);
        let header_host = Host{
            hostname: "tower.im".to_owned(),
            port: None};
        let header_ua = UserAgent("Mozilla/5.0 (X11; Linux x86_64; rv:51.0) Gecko/20100101 Firefox/51.0".to_owned());

        // set headers
        {
            let mut headers = &mut self.headers;
            headers.set(header_host);
            headers.set(header_ua);
            headers.set(header_cookie);
        }

        // members page
        let url = &self.members_url(&self.tid);
        let client = Client::new();
        let request = client.get(url).headers(self.headers.clone());
        let mut response = request.send().unwrap();
        let mut content = String::new();
        let _ = response.read_to_string(&mut content);

        // get csrf-token
        let re = Regex::new(r#"content="([^"]+)" name="csrf-token""#).unwrap();
        let caps = re.captures(&content).unwrap();
        let header_csrf_token = XCSRFToken(caps.at(1).unwrap().to_owned());
        self.headers.set(header_csrf_token);

        // get conn-guid
        let re = Regex::new(r#"id="conn-guid" value="(\w+)"#).unwrap();
        let caps = re.captures(&content).unwrap();
        self.conn_guid = caps.at(1).unwrap().to_owned();

        // get uid
        let re = Regex::new(r#"id="member-guid" value="(\w+)"#).unwrap();
        let caps = re.captures(&content).unwrap();
        self.uid = caps.at(1).unwrap().to_owned();

        // find member uid list
        let re = Regex::new(r#"href="/members/(\w+)" title="([^"]+)"#).unwrap();
        for caps in re.captures_iter(&content) {
            self.member_list.insert(caps.at(2).unwrap().to_owned(), caps.at(1).unwrap().to_owned());
        }

        true
    }

    pub fn show_weekly_reports(&self) {
        let client = Client::new();
        let url = self.weekly_reports_url(&self.uid, &self.conn_guid);
        // println!("{}", url);
        let request = client.get(&url).headers(self.headers.clone());
        let mut response = request.send().unwrap();
        let mut content = String::new();
        let _ = response.read_to_string(&mut content);
        // println!("{}", content);

        // search weekly_title
        let mut titles = Vec::new();
        let re = Regex::new(r#"<dt><i class="icon twr twr-quote-left"></i>([^<]+)</dt>"#).unwrap();
        for caps in re.captures_iter(&content) {
            titles.push(caps.at(1).unwrap());
            // println!("{}", caps.at(1).unwrap());
        }

        // search weekly_content
        let mut contents = Vec::new();
        let re = Regex::new(r#"<dd class="editor-style">(.*?)</dd>"#).unwrap();
        for caps in re.captures_iter(&content) {
            contents.push(caps.at(1).unwrap());
            // println!("{}", caps.at(1).unwrap());
        }

        assert!(titles.len() == contents.len());

        for i in 0..titles.len() {
            println!("{}", titles[i]);
            println!("{}", contents[i]);
        }
    }

    fn weekly_reports_url<T: AsRef<str>>(&self, uid: T, conn_guid: T) -> String {
        format!("https://tower.im/members/{}/weekly_reports/?conn_guid={}&pjax=1", uid.as_ref(), conn_guid.as_ref())
    }

    fn profile_url<T: AsRef<str>>(&self, uid: T) -> String {
        format!("https://tower.im/members/{}/?me=1", uid.as_ref())
    }

    fn members_url<T: AsRef<str>>(&self, tid: T) -> String {
        format!("https://tower.im/teams/{}/members/", tid.as_ref())
    }
}

fn main() {

    let mut app = App::new();
    app.load_sqlite("/home/.mozilla/firefox/f4gtaef6.default/cookies.sqlite");
    app.show_weekly_reports();
}
