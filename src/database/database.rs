

extern crate rusqlite;

use self::rusqlite::Connection;
use self::rusqlite::SQLITE_OPEN_READ_ONLY;

use std::result::Result;

pub struct SqliteCookie {
    pub path: String,
    db_type: SqliteType,

    team_id: String,
    token: String,
}

#[derive(Clone, Debug)]
pub enum SqliteType {
    Firefox,
    Chrome,
}

impl SqliteCookie {
    pub fn new<T: AsRef<str>>(p: T, db_type: SqliteType) -> SqliteCookie {
        SqliteCookie {
            path: p.as_ref().to_owned(),
            db_type: db_type,
            team_id: String::new(),
            token: String::new(),
        }
    }

    pub fn read_data(&mut self) -> Result<(), ()> {
        match self.db_type.clone() {
            SqliteType::Firefox => self.read_firefox(),
            SqliteType::Chrome => self.read_chrome(),
        }
    }

    fn read_chrome(&mut self) -> Result<(), ()> {
        println!("{}", self.path);
        Ok(())
    }

    fn read_firefox(&mut self) -> Result<(), ()> {
        let connection_flag = SQLITE_OPEN_READ_ONLY;
        let connection = Connection::open_with_flags(self.path.clone(), connection_flag).unwrap();

        let mut stmt =
            connection.prepare("select baseDomain, name, value from moz_cookies where baseDomain = \
                          'tower.im' or baseDomain = '.tower.im'")
                .unwrap();
        let mut rows = stmt.query(&[]).unwrap();

        while let Some(res) = rows.next() {
            let row = res.unwrap();

            let name: String = row.get(1);
            let value: String = row.get(2);

            match name.as_ref() {
                "remember_team_guid" => self.team_id = value,
                "remember_token" => self.token = value,
                _ => {}
            }
        }

        Ok(())
    }

    pub fn team_id(&self) -> &String {
        &self.team_id
    }

    pub fn token(&self) -> &String {
        &self.token
    }
}
