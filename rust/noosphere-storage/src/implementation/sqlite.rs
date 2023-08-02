use super::channel::{message_channel, Message, MessageClient, MessageProcessor};
use crate::storage::Storage;
use crate::store::Store;
use async_trait::async_trait;
use sqlite::SqliteClient;
use std::{path::PathBuf, sync::Arc};
use tokio::task;

pub enum SqliteStorageInit {
    Path(PathBuf),
    Db(),
}

#[derive(Clone, Debug)]
pub struct SqliteStorage {
    client: SqliteClient,
}

impl SqliteStorage {
    pub fn new(init: SqliteStorageInit) -> anyhow::Result<Self> {
        let client = match init {
            SqliteStorageInit::Path(path) => SqliteClient::new(path),
            SqliteStorageInit::Db() => panic!("unsupported"),
        };

        Ok(SqliteStorage { client })
    }

    async fn get_store(&self, name: &str) -> anyhow::Result<SqliteStore> {
        SqliteStore::new(&self.client, name).await
    }
}

#[async_trait]
impl Storage for SqliteStorage {
    type BlockStore = SqliteStore;

    type KeyValueStore = SqliteStore;

    async fn get_block_store(&self, name: &str) -> anyhow::Result<Self::BlockStore> {
        self.get_store(name).await
    }

    async fn get_key_value_store(&self, name: &str) -> anyhow::Result<Self::KeyValueStore> {
        self.get_store(name).await
    }
}

#[derive(Clone)]
pub struct SqliteStore {
    client: SqliteClient,
    table_name: String,
}

impl SqliteStore {
    async fn new(client: &SqliteClient, table_name: &str) -> anyhow::Result<Self> {
        client.create(table_name).await?;
        Ok(SqliteStore {
            client: client.to_owned(),
            table_name: table_name.to_owned(),
        })
    }

    // Try to interpret key bytes as a Cid. If that fails, try to
    // interpret bytes as UTF-8.
    fn key_bytes_to_key(key: &[u8]) -> anyhow::Result<String> {
        match cid::Cid::try_from(key) {
            Ok(cid) => Ok(cid.to_string()),
            Err(_) => match String::from_utf8(key.to_owned()) {
                Ok(string) => Ok(string),
                Err(_) => Err(anyhow::anyhow!("Could not serialize sqlite key.")),
            },
        }
    }
}

#[async_trait]
impl Store for SqliteStore {
    async fn read(&self, key: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
        self.client
            .read(&self.table_name, SqliteStore::key_bytes_to_key(key)?)
            .await
    }

    async fn write(&mut self, key: &[u8], bytes: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
        self.client
            .write(
                &self.table_name,
                SqliteStore::key_bytes_to_key(key)?,
                bytes.to_owned(),
            )
            .await
    }

    async fn remove(&mut self, key: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
        self.client
            .remove(&self.table_name, SqliteStore::key_bytes_to_key(key)?)
            .await
    }

    async fn flush(&self) -> anyhow::Result<()> {
        self.client.flush(&self.table_name).await
    }
}

mod sqlite {
    use super::*;
    use rusqlite::{params, Connection, OptionalExtension, Transaction};
    use std::result::Result;
    use tokio::fs;

    #[derive(Clone, Debug)]
    enum SqliteCommand {
        Create {
            table_name: String,
        },
        Read {
            table_name: String,
            key: String,
        },
        Write {
            table_name: String,
            key: String,
            value: Vec<u8>,
        },
        Remove {
            table_name: String,
            key: String,
        },
        Flush {
            table_name: String,
        },
    }

    #[derive(Clone, Debug)]
    enum SqliteResponse {
        Value(Option<Vec<u8>>),
        Empty,
    }

    #[derive(Debug)]
    pub struct SqliteClient {
        sender: MessageClient<SqliteCommand, SqliteResponse, anyhow::Error>,
        handle: Arc<task::JoinHandle<anyhow::Result<()>>>,
    }

    // Manually implement clone so that MessageChannel<Q,S,E>'s generics don't
    // need to also implement.
    impl Clone for SqliteClient {
        fn clone(&self) -> Self {
            SqliteClient {
                sender: self.sender.clone(),
                handle: self.handle.clone(),
            }
        }
    }

    impl SqliteClient {
        /// Create a new [SqliteClient], opening a database at `path`, creating
        /// a background thread processor.
        pub fn new(path: PathBuf) -> Self {
            let (sender, receiver) = message_channel();
            let handle = Arc::new(task::spawn(async move {
                sqlite::sqlite_task(path, receiver).await
            }));
            SqliteClient { handle, sender }
        }

        pub async fn read(&self, table_name: &str, key: String) -> anyhow::Result<Option<Vec<u8>>> {
            match self
                .sender
                .send(SqliteCommand::Read {
                    table_name: table_name.to_owned(),
                    key,
                })
                .await??
            {
                SqliteResponse::Value(value) => Ok(value),
                _ => Err(anyhow::anyhow!("Invalid sqlite response.")),
            }
        }

        pub async fn write(
            &self,
            table_name: &str,
            key: String,
            value: Vec<u8>,
        ) -> anyhow::Result<Option<Vec<u8>>> {
            match self
                .sender
                .send(SqliteCommand::Write {
                    table_name: table_name.to_owned(),
                    key,
                    value,
                })
                .await??
            {
                SqliteResponse::Value(value) => Ok(value),
                _ => Err(anyhow::anyhow!("Invalid sqlite response.")),
            }
        }

        pub async fn remove(
            &self,
            table_name: &str,
            key: String,
        ) -> anyhow::Result<Option<Vec<u8>>> {
            match self
                .sender
                .send(SqliteCommand::Remove {
                    table_name: table_name.to_owned(),
                    key,
                })
                .await??
            {
                SqliteResponse::Value(value) => Ok(value),
                _ => Err(anyhow::anyhow!("Invalid sqlite response.")),
            }
        }

        pub async fn create(&self, table_name: &str) -> anyhow::Result<()> {
            match self
                .sender
                .send(SqliteCommand::Create {
                    table_name: table_name.to_owned(),
                })
                .await??
            {
                SqliteResponse::Empty => Ok(()),
                _ => Err(anyhow::anyhow!("Invalid sqlite response.")),
            }
        }

        pub async fn flush(&self, table_name: &str) -> anyhow::Result<()> {
            match self
                .sender
                .send(SqliteCommand::Flush {
                    table_name: table_name.to_owned(),
                })
                .await??
            {
                SqliteResponse::Empty => Ok(()),
                _ => Err(anyhow::anyhow!("Invalid sqlite response.")),
            }
        }
    }

    fn create_table(tx: &Transaction, table_name: &str) -> Result<(), rusqlite::Error> {
        let create_stmt = format!(
            "CREATE TABLE IF NOT EXISTS {} (
                        _id INTEGER PRIMARY KEY,
                        key TEXT NOT NULL UNIQUE,
                        value BLOB
                )",
            table_name
        );
        tx.prepare_cached(&create_stmt)?.execute(())?;
        Ok(())
    }

    fn read_value(
        tx: &Transaction,
        table_name: &str,
        key: &str,
    ) -> Result<Option<Vec<u8>>, rusqlite::Error> {
        let read_stmt = format!("SELECT VALUE FROM {} WHERE key = $1", table_name);
        Ok(tx
            .prepare_cached(&read_stmt)?
            .query_row(params![key], |r| r.get(0))
            .optional()?)
    }

    fn write_value(
        tx: &Transaction,
        table_name: &str,
        key: &str,
        value: &[u8],
    ) -> Result<Option<Vec<u8>>, rusqlite::Error> {
        let write_stmt = format!(
            "INSERT INTO {} (key, value)
            VALUES($1, $2) 
            ON CONFLICT(key) 
            DO UPDATE SET value=excluded.value",
            table_name
        );
        let previous = read_value(&tx, table_name, key)?;
        tx.prepare_cached(&write_stmt)?
            .execute(params![key, value])?;
        Ok(previous)
    }

    fn remove_value(
        tx: &Transaction,
        table_name: &str,
        key: &str,
    ) -> Result<Option<Vec<u8>>, rusqlite::Error> {
        let delete_stmt = format!("DELETE FROM {} WHERE key = $1", table_name);
        let previous = read_value(&tx, table_name, key)?;
        tx.prepare_cached(&delete_stmt)?.execute(params![key])?;
        Ok(previous)
    }

    async fn sqlite_task(
        path: PathBuf,
        mut rx: MessageProcessor<SqliteCommand, SqliteResponse, anyhow::Error>,
    ) -> anyhow::Result<()> {
        // `Connection::open` requires the ancestor directories to exist.
        tokio::fs::create_dir_all(&path).await?;
        let mut db = Connection::open(path.join("database.db"))?;

        while let Some(request) = rx.pull_message().await {
            let response = process_message(&mut db, &request.request);
            request.respond(response.map_err(|e| e.into()));
        }
        Ok(())
    }

    fn process_message(
        db: &mut Connection,
        command: &SqliteCommand,
    ) -> Result<SqliteResponse, rusqlite::Error> {
        match command {
            SqliteCommand::Flush { table_name } => Ok(SqliteResponse::Empty),
            SqliteCommand::Create { table_name } => {
                let tx = db.transaction()?;
                create_table(&tx, &table_name)?;
                tx.commit()?;
                Ok(SqliteResponse::Empty)
            }
            SqliteCommand::Read { table_name, key } => {
                let tx = db.transaction()?;
                let value = read_value(&tx, &table_name, &key)?;
                tx.commit()?;
                Ok(SqliteResponse::Value(value))
            }
            SqliteCommand::Write {
                table_name,
                key,
                value,
            } => {
                let tx = db.transaction()?;
                let previous = write_value(&tx, &table_name, &key, &value)?;
                tx.commit()?;
                Ok(SqliteResponse::Value(previous))
            }
            SqliteCommand::Remove { table_name, key } => {
                let tx = db.transaction()?;
                let previous = remove_value(&tx, &table_name, &key)?;
                tx.commit()?;
                Ok(SqliteResponse::Value(previous))
            }
        }
    }
}
