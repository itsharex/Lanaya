use crate::utils::dirs::app_data_dir;
use crate::utils::string_util;
use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

#[derive(serde::Serialize, serde::Deserialize, Debug, Default, PartialEq)]
pub struct Record {
    pub id: u64,
    pub content: String,
    pub md5: String,
    pub create_time: u64,
    pub is_favorite: bool,
    // 仅在搜索返回时使用
    pub content_highlight: Option<String>,
}

pub struct SqliteDB {
    conn: Connection,
}

const SQLITE_FILE: &str = "data.sqlite";

#[allow(unused)]
impl SqliteDB {
    pub fn new() -> Self {
        let data_dir = app_data_dir().unwrap().join(SQLITE_FILE);
        let c = Connection::open_with_flags(data_dir, OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
        SqliteDB { conn: c }
    }
    pub fn add(&self) -> i64 {
        self.conn.last_insert_rowid()
    }
    pub fn init() {
        let data_dir = app_data_dir().unwrap().join(SQLITE_FILE);
        if !Path::new(&data_dir).exists() {
            File::create(&data_dir).unwrap();
        }
        let c = Connection::open_with_flags(data_dir, OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
        let sql = r#"
        create table if not exists record
        (
            id          INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
            content     TEXT,
            md5         VARCHAR(200) DEFAULT '',
            create_time INTEGER,
            is_favorite INTEGER DEFAULT 0
        );
        "#;
        c.execute(sql, ()).unwrap();
    }

    pub fn insert_record(&self, r: Record) -> Result<i64> {
        let sql = "insert into record (content,md5,create_time,is_favorite) values (?1,?2,?3,?4)";
        let md5 = string_util::md5(r.content.as_str());
        let now = chrono::Local::now().timestamp_millis() as u64;
        let res = self
            .conn
            .execute(sql, (&r.content, md5, now, &r.is_favorite))?;
        Ok(self.conn.last_insert_rowid())
    }

    fn find_record_by_md5(&self, md5: String) -> Result<Record> {
        let sql = "SELECT id, content, md5, create_time, is_favorite FROM record WHERE md5 = ?1";
        let r = self.conn.query_row(sql, [md5], |row| {
            Ok(Record {
                id: row.get(0)?,
                ..Default::default()
            })
        })?;
        Ok(r)
    }

    // 更新时间
    fn update_record_create_time(&self, r: Record) -> Result<()> {
        let sql = "update record set create_time = ?2 where id = ?1";
        // 获取当前毫秒级时间戳
        let now = chrono::Local::now().timestamp_millis() as u64;
        self.conn.execute(sql, [&r.id, &now])?;
        Ok(())
    }

    pub fn insert_if_not_exist(&self, r: Record) -> Result<()> {
        let md5 = string_util::md5(r.content.as_str());
        match self.find_record_by_md5(md5) {
            Ok(res) => {
                self.update_record_create_time(res)?;
            }
            Err(_e) => {
                self.insert_record(r)?;
            }
        }
        Ok(())
    }

    // 清除数据
    pub fn clear_data(&self) -> Result<()> {
        let sql = "delete from record";
        self.conn.execute(sql, ())?;
        Ok(())
    }

    // 标记为收藏
    pub fn mark_favorite(&self, id: u64) -> Result<()> {
        let sql = "update record set is_favorite = 1 where id = ?1";
        self.conn.execute(sql, [&id])?;
        Ok(())
    }

    pub fn find_all(&self) -> Result<Vec<Record>> {
        let sql = "SELECT id, content, md5, create_time, is_favorite FROM record order by create_time desc";
        let mut stmt = self.conn.prepare(sql)?;
        let mut rows = stmt.query([])?;
        let mut res = vec![];
        while let Some(row) = rows.next()? {
            let r = Record {
                id: row.get(0)?,
                content: row.get(1)?,
                md5: row.get(2)?,
                create_time: row.get(3)?,
                is_favorite: row.get(4)?,
                content_highlight: None,
            };
            res.push(r);
        }
        Ok(res)
    }

    pub fn find_by_key(&self, key: String, limit: u64) -> Result<Vec<Record>> {
        let sql = "SELECT id, content, md5, create_time, is_favorite FROM record where content like ?1 order by create_time desc limit ?2";
        let mut stmt = self.conn.prepare(sql)?;
        let mut rows = stmt.query([format!("%{}%", key), limit.to_string()])?;
        let mut res = vec![];
        while let Some(row) = rows.next()? {
            let content: String = row.get(1)?;
            let content_highlight = Some(string_util::highlight(key.as_str(), content.as_str()));
            let r = Record {
                id: row.get(0)?,
                content,
                md5: row.get(2)?,
                create_time: row.get(3)?,
                is_favorite: row.get(4)?,
                content_highlight: None,
            };
            res.push(r);
        }
        Ok(res)
    }

    fn find_by_id_in(&self, ids: Vec<u64>) -> Result<Vec<Record>> {
        let ids_string = ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<String>>()
            .join(",");
        let sql = format!(
            "SELECT id, content, md5, create_time, is_favorite FROM record where id in ({})",
            ids_string
        );
        let mut stmt = self.conn.prepare(sql.as_str())?;
        let mut rows = stmt.query([])?;
        let mut res = vec![];
        while let Some(row) = rows.next()? {
            let r = Record {
                id: row.get(0)?,
                content: row.get(1)?,
                md5: row.get(2)?,
                create_time: row.get(3)?,
                is_favorite: row.get(4)?,
                content_highlight: None,
            };
            res.push(r);
        }
        Ok(res)
    }

    //删除超过limit的记录
    pub fn delete_over_limit(&self, limit: usize) -> Result<()> {
        // 先查询count，如果count - limit > 50 就删除 超出limit部分记录 主要是防止频繁重建数据库
        let stmt = self.conn.prepare("SELECT count(*) FROM record")?;
        let count = stmt.column_count();
        if count < 50 + limit {
            return Ok(());
        }
        let sql = "DELETE FROM record WHERE id IN (SELECT id FROM record ORDER BY id DESC LIMIT ?1, 1000000000)";
        self.conn.execute(sql, [&limit])?;
        Ok(())
    }
}

#[test]
fn test_sqlite_insert() {
    SqliteDB::init();
    let r = Record {
        content: "123456".to_string(),
        md5: "e10adc3949ba59abbe56e057f20f883e".to_string(),
        create_time: 1234568,
        ..Default::default()
    };
    assert_eq!(SqliteDB::new().insert_record(r).unwrap(), 1_i64)
}

#[test]
fn test_find_by_md5() {
    // SqliteDB::init();
    // let a = SqliteDB::new().find_all().unwrap();

    // println!("{:?}", a);

    let b = SqliteDB::new().find_by_key("r".to_string(), 10).unwrap();
    println!("{:?}", b);
}
