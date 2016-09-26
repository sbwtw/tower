
#[macro_use]
extern crate log;
extern crate env_logger;
#[macro_use]
extern crate hyper;
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

use clap::{Arg, App};

use time::*;

use rustc_serialize::json::*;

use database::SqliteCookie;

use hyper::client::*;
use hyper::header::*;

use regex::Regex;

header! { (XCSRFToken, "X-CSRF-Token") => [String] }
header! { (POSTAccept, "Accept") => [String] }

struct Tower {
    client: Client,
    tid: String,
    uid: String,
    conn_guid: String,
    answers: Vec<String>,
    headers: Headers,
    member_list: HashMap<String, String>,
}

impl Tower {
    pub fn new() -> Tower {
        Tower {
            client: Client::new(),
            tid: String::new(),
            uid: String::new(),
            conn_guid: String::new(),
            answers: Vec::<String>::new(),
            headers: Headers::new(),
            member_list: HashMap::with_capacity(200),
        }
    }

    pub fn load_sqlite<T: AsRef<str>>(&mut self, file: T) -> bool {

        debug!("load sqlite from: {}", file.as_ref());

        let mut sc = SqliteCookie::new(file);
        let _ = sc.read_data();

        // set headers
        self.tid = sc.team_id().clone();

        let token = sc.token();

        let header_cookie = Cookie(vec![
                CookiePair::new("remember_team_guid".to_owned(), self.tid.clone()),
                CookiePair::new("remember_token".to_owned(), token.clone()),
            ]);
        let header_host = Host{hostname: "tower.im".to_owned(), port: None};
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
        let request = self.client.get(url).headers(self.headers.clone());
        let mut response = request.send().unwrap();
        let mut content = String::new();
        let _ = response.read_to_string(&mut content);

        // get extra cookies
        for cookie in &response.headers.get::<SetCookie>().unwrap().0 {
            if cookie.name == "_tower2_session" {
                self.headers.get_mut::<Cookie>().unwrap().push(cookie.clone());
                break;
            }
        }

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
    }

    pub fn send_weekly_reports(&mut self) {

        let year = self.current_year();
        let week = self.current_week();

        // get weekly info
        let url = format!("https://tower.im/members/{}/weekly_reports/{}-{}/edit?conn_guid={}",
                            self.uid, year, week, self.conn_guid);

        let mut result = String::new();
        {
            let request = self.client.get(&url).headers(self.headers.clone());
            let mut response = request.send().unwrap();
            let _ = response.read_to_string(&mut result);
        }

        let json: Json = result.parse().unwrap();
        let result = json["html"].as_string().unwrap();

        let mut fields = Vec::<(&str, &str, &str)>::new();
        let re = Regex::new(r#"<input.*?name="(.*?)".*?value="(.*?)".*?>\s*(.*?)\s*</div>"#).unwrap();
        for caps in re.captures_iter(&result) {
            let k = caps.at(1).unwrap();
            let v = caps.at(2).unwrap();
            let t = caps.at(3).unwrap();
            fields.push((k, v, t));
        }

        // check answers match fields
        while !self.confirm_answers(&fields) {
            self.get_weekly_answers(&fields);
        }

        let mut answers = Array::new();
        for (i, ans) in self.answers.iter().enumerate() {

            let mut object = Object::new();
            object.insert("content".to_owned(), Json::String(ans.to_owned()));
            object.insert(fields[i].0.to_owned(), Json::String(fields[i].1.to_owned()));

            answers.push(Json::Object(object));
        }

        let answers = encode(&answers).unwrap();
        let send_data = format!("conn_guid={}&data={}", self.conn_guid, answers);

        let mut headers = self.headers.clone();
        headers.set(POSTAccept("application/json, text/javascript, */*; q=0.01".to_owned()));

        let url = format!("https://tower.im/members/{}/weekly_reports/{}-{}",
                            self.uid, year, week);
        let request = self.client.post(&url).body(&send_data).headers(headers);
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
            titles.push(caps.at(1).unwrap().to_owned());
        }

        // search weekly_content
        let mut contents = Vec::new();
        let re = Regex::new(r#"<dd class="editor-style">(.*?)</dd>"#).unwrap();
        for caps in re.captures_iter(&content) {
            contents.push(caps.at(1).unwrap().to_owned());
        }

        assert!(titles.len() == contents.len());

        (titles, contents)
    }

    fn confirm_answers(&self, fields: &Vec<(&str, &str, &str)>) -> bool {
        assert!(fields.len() >= self.answers.len());

        print!("\n");

        // print user answers
        for (i, ref answer) in self.answers.iter().enumerate() {
            println!("{}:\n{}\n\n", fields[i].2, answer);
        }

        if fields.len() != self.answers.len() {
            println!("some fields not filled:");

            for i in self.answers.len()..fields.len() {
                println!("{}", fields[i].2);
            }
        }

        ask_question("Submit your answers?", true)
    }

    fn get_weekly_answers(&mut self, fields: &Vec<(&str, &str, &str)>) {

        print!("\n");

        let exist_len = self.answers.len();
        for (i, &(_, _, title)) in fields.iter().enumerate() {
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

        assert!(fields.len() == self.answers.len())
    }

    fn weekly_reports_url<T: AsRef<str>>(&self, uid: T, conn_guid: T) -> String {
        format!("https://tower.im/members/{}/weekly_reports/?conn_guid={}&pjax=1", uid.as_ref(), conn_guid.as_ref())
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
}

fn ask_question<T: AsRef<str>>(q: T, default: bool) -> bool {

    if default == true {
        print!("{} [Y/n]:", q.as_ref());
    } else {
        print!("{} [y/N]:", q.as_ref());
    }
    let _  = stdout().flush();

    let mut result = String::new();
    stdin().read_line(&mut result).unwrap();
    let result = result.trim().to_lowercase();

    match result.as_str() {
        "y" | "yes" => true,
        "n" | "no"  => false,
        _           => default,
    }
}

fn search_cookie_sqlite() -> Option<String> {

    let home = home_dir().unwrap();
    let config_file = format!("{}/.mozilla/firefox/profiles.ini", home.display());

    let file = File::open(config_file);
    if file.is_err() {
        return None;
    }

    for line in BufReader::new(&file.unwrap()).lines() {
        if line.is_err() {continue;}

        let l = line.unwrap();
        if !l.starts_with("Path=") {continue;}

        let dir: Vec<&str> = l.split('=').collect();
        let sqlite = format!("{}/.mozilla/firefox/{}/cookies.sqlite", home.display(), dir[1]);
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
                    .arg(Arg::with_name("send")
                         .short("s")
                         .long("send")
                         .help("Send your weekly reports"))
                    .arg(Arg::with_name("today")
                         .short("t")
                         .long("today")
                         .conflicts_with("send")
                         .help("Send your today reports"))
                    //.arg(Arg::with_name("reports")
                         //.short("r")
                         //.long("reports")
                         //.takes_value(true)
                         //.default_value("")
                         //.help("Your reports content"))
                    .get_matches();

    env_logger::init().unwrap();

    let mut tower = Tower::new();
    if let Some(file) = search_cookie_sqlite() {
        tower.load_sqlite(file);
    } else {
        panic!("cant load cookies");
    }

    //if matches.is_present("reports") {
        //println!("{:?}", matches.value_of("reports"));
    //}

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
}
