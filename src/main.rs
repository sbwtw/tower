
#[macro_use]
extern crate log;
extern crate env_logger;
#[macro_use]
extern crate hyper;
extern crate hyper_native_tls;
extern crate regex;
extern crate time;
extern crate rustc_serialize;
extern crate clap;

mod database;

use std::io::*;
use std::env::*;
use std::fs::File;
use std::path::Path;
use std::collections::HashMap;
use std::process::Command;
use std::os::unix::process::CommandExt;

use clap::{Arg, App};

use time::*;

use rustc_serialize::json::*;

use database::SqliteCookie;
use database::SqliteType;

use hyper::client::*;
use hyper::header::*;
use hyper::status::StatusCode;
use hyper::net::HttpsConnector;
use hyper_native_tls::NativeTlsClient;

use regex::Regex;

header! { (XCSRFToken, "X-CSRF-Token") => [String] }
header! { (POSTAccept, "Accept") => [String] }

struct Tower {
    client: Client,
    tid: String,
    uid: String,
    conn_guid: String,
    weekly_info: Vec<(String, String, String)>,
    answers: Vec<String>,
    headers: Headers,
    member_list: HashMap<String, String>,
    disable_confirm: bool,
}

impl Tower {
    pub fn new() -> Tower {
        Tower {
            client: Client::with_connector(HttpsConnector::new(NativeTlsClient::new().unwrap())),
            tid: String::new(),
            uid: String::new(),
            conn_guid: String::new(),
            weekly_info: Vec::<(String, String, String)>::new(),
            answers: Vec::<String>::new(),
            headers: Headers::new(),
            member_list: HashMap::with_capacity(200),
            disable_confirm: false,
        }
    }

    pub fn load_sqlite<T: AsRef<str>>(&mut self, file: T, db_type: SqliteType) -> bool {

        debug!("load sqlite from: {}", file.as_ref());

        let mut sc = SqliteCookie::new(file, db_type);
        let _ = sc.read_data();

        // set headers
        self.tid = sc.team_id().clone();

        let token = sc.token();

        let mut header_cookie = Cookie(vec![]);
        header_cookie.0.push(format!("remember_team_guid={}", self.tid));
        header_cookie.0.push(format!("remember_token={}", token));

        let header_host = Host {
            hostname: "tower.im".to_owned(),
            port: None,
        };
        let header_ua = UserAgent("Mozilla/5.0 (X11; Linux x86_64; rv:51.0) Gecko/20100101 \
                                   Firefox/51.0"
            .to_owned());

        // set headers
        {
            let mut headers = &mut self.headers;
            headers.set(header_host);
            headers.set(header_ua);
            headers.set(header_cookie);
        }

        // members page
        let url = &self.members_url(&self.tid);
        let request = self.client.get(url).headers(self.headers.clone());
        let mut response = request.send().unwrap();
        let mut content = String::new();
        let _ = response.read_to_string(&mut content);

        // get extra cookies
        for cookie in &response.headers.get::<SetCookie>().unwrap().0 {
            if cookie.starts_with("_tower2_session") {
                let i = cookie.split(';').next().unwrap();
                info!("find cookie: {}", i);
                self.headers.get_mut::<Cookie>().unwrap().push(i.to_owned());
                break;
            }
        }

        // get csrf-token
        let re = Regex::new(r#"content="([^"]+)" name="csrf-token""#).unwrap();
        let caps = re.captures(&content).unwrap();
        let header_csrf_token = XCSRFToken(caps.get(1).unwrap().as_str().to_owned());
        self.headers.set(header_csrf_token);

        // get conn-guid
        let re = Regex::new(r#"id="conn-guid" value="(\w+)"#).unwrap();
        let caps = re.captures(&content).unwrap_or_else(|| panic!("find conn-guid failed"));
        self.conn_guid = caps.get(1).unwrap().as_str().to_owned();

        // get uid
        let re = Regex::new(r#"id="member-guid" value="(\w+)"#).unwrap();
        let caps = re.captures(&content).unwrap_or_else(|| panic!("find member-guid failed"));
        self.uid = caps.get(1).unwrap().as_str().to_owned();

        // find member uid list
        let re = Regex::new(r#"href="/members/(\w+)"[\s\S]+?member-nickname">([^<]+)"#).unwrap();
        for caps in re.captures_iter(&content) {
            let name = caps.get(2).unwrap().as_str().to_owned();
            let uid = caps.get(1).unwrap().as_str().to_owned();

            info!("got member: {} {}", name, uid);
            self.member_list.insert(name, uid);
        }

        true
    }

    pub fn show_weekly_reports(&self) {

        let (titles, contents) = self.get_weekly_reports();

        if titles.len() == 0 {
            println!("your weekly reports is empty.");
            return;
        }

        for i in 0..titles.len() {
            println!("{}", titles[i]);
            println!("{}", contents[i]);
        }
    }

    pub fn show_calendar_info(&self) {

        let url = format!("https://tower.im/members/{}/calendar_events/", self.uid);
        let request = self.client.get(&url).headers(self.headers.clone());
        let mut response = request.send().unwrap();
        let mut content = String::new();
        let _ = response.read_to_string(&mut content);

        // TODO: process content
        println!("{}", content);
        // println!("{:?}", self.current_time_formatted());
    }

    pub fn send_weekly_reports(&mut self) {

        if self.weekly_info.is_empty() {
            self.load_weekly_info();
        }

        let year = self.current_year();
        let week = self.current_week();

        // check answers match fields
        while self.answers.is_empty() || !self.confirm_answers() {
            self.get_weekly_answers();
        }

        let mut answers = Array::new();
        for (i, ans) in self.answers.iter().enumerate() {

            let mut object = Object::new();
            object.insert("content".to_owned(), Json::String(ans.to_owned()));
            object.insert(self.weekly_info[i].0.clone(),
                          Json::String(self.weekly_info[i].1.clone()));

            answers.push(Json::Object(object));
        }

        let answers = encode(&answers).unwrap();
        let send_data = format!("conn_guid={}&data={}", self.conn_guid, answers);

        let mut headers = self.headers.clone();
        headers.set(POSTAccept("application/json, text/javascript, */*; q=0.01".to_owned()));
        self.headers = headers;

        let url = format!("https://tower.im/members/{}/weekly_reports/{}-{}",
                          self.uid,
                          year,
                          week);
        let request = self.client.post(&url).body(&send_data).headers(self.headers.clone());
        let mut response = request.send().unwrap();
        let mut result = String::new();
        let _ = response.read_to_string(&mut result);
        // println!("{}", result);
        // let json: Json::Object = decode(&result).unwrap();
        let json: Json = result.parse().unwrap();
        let json = json.as_object().unwrap();

        if json.contains_key("success") && json["success"] == Json::Boolean(true) {
            println!("Post weekly report success.");
        } else {
            debug!("{}", result);
            println!("Post weekly report fail.")
        }

        self.show_weekly_reports();
    }

    pub fn send_today_reports(&mut self) {

        let day_of_week: usize = strftime("%u", &now()).unwrap().parse().unwrap();
        self.send_day_reports(day_of_week - 1);
    }

    pub fn send_overtime_record<T: AsRef<str>>(&mut self, title: T, cc_name: T) {
        self.send_overtime_internal(title, cc_name);
    }

    pub fn send_fake_reports(&mut self) {

        self.load_weekly_info();
        self.answers = self.get_weekly_reports().1;

        for i in self.answers.iter_mut() {
            if i.is_empty() {
                i.push_str("<p></p><br/>");
            }
        }

        for _ in self.answers.len()..self.weekly_info.len() {
            self.answers.push("<p></p><br/>".to_owned());
        }

        self.send_weekly_reports();
    }

    pub fn disable_confirm(&mut self) {
        self.disable_confirm = true;
    }

    fn send_overtime_internal<T: AsRef<str>>(&mut self, title: T, cc_name: T) {
        let cc_name = cc_name.as_ref();
        let cc_guid = match self.member_list.get(cc_name) {
            Some(guid) => guid,
            _ => {
                println!("User {} not exist!", cc_name);
                return;
            }
        };

        let day_string = strftime("%Y-%m-%d", &now()).unwrap();
        let (cur_hour, cur_min) = self.current_time_formatted();

        let post_url = format!("https://tower.im/teams/{}/calendar_events/", self.tid);
        let start_time = format!("{}+17%3A30%3A00", day_string);
        let end_time = format!("{}+{}%3A{}%3A00", day_string, cur_hour, cur_min);
        let post_body = format!("conn_guid={}&content={}&starts_at={}&ends_at={}&is_show_creator=true&caleventable_type=Calendar&caleventable_guid=b96e5a357a884c7e8c5c2ab12858dd02&schedule_interval=1",
                                self.conn_guid,
                                title.as_ref(),
                                start_time,
                                end_time);

        // let post_body = format!("conn_guid={}&content={}&starts_at={}&ends_at={}&schedule_until={}&schedule_every=0&location=&remind_time=&is_show_creator=true&member_guids=&caleventable_type=Calendar&caleventable_guid=b96e5a357a884c7e8c5c2ab12858dd02&start=2016-10-31&end=2016-12-04&schedule_interval=1", self.conn_guid, content, start_time, end_time, schedule_until);

        // println!("{}, {}", start_time, end_time);

        // let post_body = format!("conn_guid={}&content={}&starts_at=2017-01-06+09%3A00%3A00&ends_at=2017-01-06+17%3A00%3A00&schedule_until=2017-01-06+23%3A59%3A59&schedule_every=0&location=&remind_time=&is_show_creator=true&member_guids=&caleventable_type=Calendar&caleventable_guid=b96e5a357a884c7e8c5c2ab12858dd02&start=2016-12-26&end=2017-02-05&schedule_interval=1",
        //                     self.conn_guid,
        //                     title.as_ref()
        //                     );

        // post data and check result
        let response = self.post_data(post_url, post_body);
        let result: Json = response.parse().unwrap();
        let object = result.as_object().unwrap();

        if object.get("success") != Some(&Json::Boolean(true)) {
            info!("post overtime error:");
            info!("{}", result);
            return;
        }

        if let Some(&Json::String(ref url)) = object.get("url") {
            // let content = "conn_guid=51586e27839ff9f7766f16bad29b49c4&comment_content=%3Cp%3E%3Ca+href%3D%22%2Fmembers%2F83555be08c1a4912a1f875636afa3f52%22+data-mention%3D%22true%22%3E%40%E5%BC%A0%E7%BB%A7%E5%BE%B7%3C%2Fa%3E%26nbsp%3B%3Cbr%3E%3C%2Fp%3E&is_html=1&cc_guids=83555be08c1a4912a1f875636afa3f52";

            let comment_content = format!("<p><a href=\"/members/{}\" \
                                           data-mention=\"true\">@{}</a></p>",
                                          cc_guid,
                                          cc_name);
            let content = format!("conn_guid={}&comment_content={}&is_html=1&cc_guids={}",
                                  self.conn_guid,
                                  comment_content,
                                  self.member_list.get(cc_name).unwrap());

            let _ = self.post_data(format!("https://tower.im{}/comments", url), content);

            let url = format!("https://tower.im{}", url);
            println!("send overtime finished, url is {}", url);

            Command::new("gio")
                .arg("open")
                .arg(url)
                .exec();
        } else {
            println!("send overtime failed.");
        }
    }

    fn post_data<U: AsRef<str>, B: AsRef<str>>(&self, url: U, body: B) -> String {
        let request =
            self.client.post(url.as_ref()).body(body.as_ref()).headers(self.headers.clone());
        let mut response = request.send().unwrap();
        assert_eq!(StatusCode::Ok, response.status);
        let mut result = String::new();
        let _ = response.read_to_string(&mut result);

        result
    }

    fn get_data<T: AsRef<str>>(&self, url: T) -> String {
        let req = self.client.get(url.as_ref()).headers(self.headers.clone());
        let mut response = req.send().unwrap();
        assert_eq!(StatusCode::Ok, response.status);
        let mut result = String::new();
        let _ = response.read_to_string(&mut result);

        result
    }

    fn load_weekly_info(&mut self) {

        assert!(self.weekly_info.is_empty());

        let year = self.current_year();
        let week = self.current_week();

        // get weekly info
        let url = format!("https://tower.im/members/{}/weekly_reports/{}-{}/edit?conn_guid={}",
                          self.uid,
                          year,
                          week,
                          self.conn_guid);

        let result = self.get_data(&url);
        let json: Json = result.parse().unwrap();
        let result = json["html"].as_string().unwrap();

        let ref mut fields = self.weekly_info;
        // let mut fields = Vec::<(&str, &str, &str)>::new();
        let re = Regex::new(r#"<input.*?name="(.*?)".*?value="(.*?)".*?>\s*(.*?)\s*</div>"#)
            .unwrap();
        for caps in re.captures_iter(&result) {
            let k = caps.get(1).unwrap().as_str();
            let v = caps.get(2).unwrap().as_str();
            let t = caps.get(3).unwrap().as_str();
            fields.push((k.to_owned(), v.to_owned(), t.to_owned()));
        }
    }

    // send spec day report, index is start with 0
    fn send_day_reports(&mut self, index: usize) {

        println!("input your reports of day {}", index + 1);
        let mut ans = String::new();
        let _ = stdin().read_to_string(&mut ans);

        let (_, mut answers) = self.get_weekly_reports();

        while answers.len() <= index {
            answers.push(String::new());
        }

        answers[index] = ans;
        self.answers = answers;
        self.send_weekly_reports();
    }

    fn get_weekly_reports(&self) -> (Vec<String>, Vec<String>) {

        let url = self.weekly_reports_url(&self.uid, &self.conn_guid);
        let request = self.client.get(&url).headers(self.headers.clone());
        let mut response = request.send().unwrap();
        let mut content = String::new();
        let _ = response.read_to_string(&mut content);

        // search weekly_title
        let mut titles = Vec::new();
        let re = Regex::new(r#"<dt><i class="icon twr twr-quote-left"></i>([^<]+)</dt>"#).unwrap();
        for caps in re.captures_iter(&content) {
            titles.push(caps.get(1).unwrap().as_str().to_owned());
        }

        // search weekly_content
        let mut contents = Vec::new();
        let re = Regex::new(r#"<dd class="editor-style">(.*?)</dd>"#).unwrap();
        for caps in re.captures_iter(&content) {
            contents.push(caps.get(1).unwrap().as_str().to_owned());
        }

        assert!(titles.len() == contents.len());

        (titles, contents)
    }

    fn confirm_answers(&self) -> bool {
        assert!(self.weekly_info.len() >= self.answers.len());

        if self.disable_confirm {
            return true;
        }

        print!("\n");

        // print user answers
        for (i, ref answer) in self.answers.iter().enumerate() {
            println!("{}:\n{}\n\n", self.weekly_info[i].2, answer);
        }

        if self.weekly_info.len() != self.answers.len() {
            println!("some fields not filled:");

            for i in self.answers.len()..self.weekly_info.len() {
                println!("{}", self.weekly_info[i].2);
            }
        }

        ask_question("Submit your answers?", !self.answers.is_empty())
    }

    fn get_weekly_answers(&mut self) {

        print!("\n");

        let exist_len = self.answers.len();
        for (i, &(_, _, ref title)) in self.weekly_info.iter().enumerate() {
            println!("\n{}:", title);

            let overflow = i >= exist_len;

            // show exist answer
            if !overflow {
                println!("default: {}", self.answers[i]);
            }

            // get user answer
            let mut answer = String::new();
            stdin().read_to_string(&mut answer).unwrap();
            let answer = answer.trim();

            let e = answer.is_empty();

            if !e && !overflow {
                self.answers[i] = answer.to_owned();
            } else if overflow {
                self.answers.push(answer.to_owned());
            }
        }

        assert!(self.weekly_info.len() == self.answers.len())
    }

    fn weekly_reports_url<T: AsRef<str>>(&self, uid: T, conn_guid: T) -> String {
        format!("https://tower.im/members/{}/weekly_reports/?conn_guid={}&pjax=1",
                uid.as_ref(),
                conn_guid.as_ref())
    }

    // fn profile_url<T: AsRef<str>>(&self, uid: T) -> String {
    //     format!("https://tower.im/members/{}/?me=1", uid.as_ref())
    // }

    fn members_url<T: AsRef<str>>(&self, tid: T) -> String {
        format!("https://tower.im/teams/{}/members/", tid.as_ref())
    }

    fn current_year(&self) -> i32 {
        now().tm_year + 1900
    }

    fn current_week(&self) -> String {
        strftime("%W", &now()).unwrap()
    }

    fn current_time_formatted(&self) -> (i32, i32) {
        let tm = now();
        let hour = tm.tm_hour;
        let minute = tm.tm_min;
        let temp_min = (minute + 15) / 30 * 30;
        let result_hour = hour + temp_min / 60;
        let result_min = temp_min % 60;

        (result_hour, result_min)
    }
}

fn ask_question<T: AsRef<str>>(q: T, default: bool) -> bool {

    if default == true {
        print!("{} [Y/n]:", q.as_ref());
    } else {
        print!("{} [y/N]:", q.as_ref());
    }
    let _ = stdout().flush();

    let mut result = String::new();
    stdin().read_line(&mut result).unwrap();
    let result = result.trim().to_lowercase();

    match result.as_str() {
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default,
    }
}

fn search_cookie_sqlite_chrome() -> Option<String> {
    None
    // Some("/home/.config/google-chrome/Profile 1/Cookies".to_owned())
}

fn search_cookie_sqlite_firefox() -> Option<String> {

    let home = home_dir().unwrap();
    let config_file = format!("{}/.mozilla/firefox/profiles.ini", home.display());

    let file = File::open(config_file);
    for line in BufReader::new(&file.unwrap()).lines() {

        if line.is_err() {
            continue;
        }

        let l = line.unwrap();
        if !l.starts_with("Path=") {
            continue;
        }

        let dir: Vec<&str> = l.split('=').collect();
        let sqlite = format!("{}/.mozilla/firefox/{}/cookies.sqlite",
                             home.display(),
                             dir[1]);
        if Path::new(&sqlite).exists() {
            return Some(sqlite);
        } else {
            return None;
        }
    }

    None
}

fn main() {

    // process command-line
    let matches = App::new("Tower")
                    .version("0.0.1")
                    .author("sbw <sbw@sbw.so>")
                    .about("Tower.im helper tools")
                    .arg(Arg::with_name("weekly")
                         .short("w")
                         .long("weekly")
                         .help("Show your weekly reports"))
                    .arg(Arg::with_name("calendar")
                         .short("c")
                         .long("calendar")
                         .help("Show your calendar info"))
                    .arg(Arg::with_name("overtime")
                         .short("o")
                         .long("overtime")
                         .requires("cc_name")
                         .help("Post overtime record"))
                    .arg(Arg::with_name("title")
                         .long("title")
                         .takes_value(true)
                         .help("Overtime list title"))
                    .arg(Arg::with_name("cc_name")
                         .long("cc")
                         .takes_value(true)
                         .help("Overtime list @somebody"))
                    .arg(Arg::with_name("send")
                         .short("s")
                         .long("send")
                         .help("Send your weekly reports"))
                    .arg(Arg::with_name("today")
                         .short("t")
                         .long("today")
                         .conflicts_with("send")
                         .help("Send your today reports"))
                    .arg(Arg::with_name("fake")
                         .short("f")
                         .long("fake")
                         .help("do some evil(add fake reports if not filled to cheat robot)."))
                    //.arg(Arg::with_name("reports")
                         //.short("r")
                         //.long("reports")
                         //.takes_value(true)
                         //.default_value("")
                         //.help("Your reports content"))
                    .arg(Arg::with_name("confirm")
                         .short("y")
                         .help("Always say yes."))
                    .get_matches();

    env_logger::init().unwrap();

    let mut tower = Tower::new();

    if let Some(file) = search_cookie_sqlite_chrome() {
        tower.load_sqlite(file, SqliteType::Chrome);
    } else if let Some(file) = search_cookie_sqlite_firefox() {
        tower.load_sqlite(file, SqliteType::Firefox);
    } else {
        panic!("can't load cookies");
    }

    if matches.is_present("confirm") {
        tower.disable_confirm();
    }

    // if matches.is_present("reports") {
    // println!("{:?}", matches.value_of("reports"));
    // }

    if matches.is_present("fake") {
        tower.send_fake_reports();
    }

    if matches.is_present("send") {
        tower.send_weekly_reports();
    }

    if matches.is_present("today") {
        tower.send_today_reports();
    }

    if matches.is_present("weekly") {
        tower.show_weekly_reports();
    }

    if matches.is_present("calendar") {
        tower.show_calendar_info();
    }

    if matches.is_present("overtime") {
        let title = matches.value_of("title").unwrap_or("加班登记");
        let cc_name = matches.value_of("cc_name").unwrap();

        tower.send_overtime_record(title, cc_name);
    }
}
