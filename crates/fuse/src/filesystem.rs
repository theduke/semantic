use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    ffi::OsStr,
    sync::Mutex,
    time::{Duration, SystemTime},
};

use bytes::Bytes;
use fuser::{
    BsdFileFlags, Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags,
    Generation, INodeNo, LockOwner, MountOption, OpenFlags, RenameFlags, ReplyAttr, ReplyCreate,
    ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyWrite, Request, SessionACL,
    TimeOrNow, WriteFlags,
};
use futures::SinkExt as _;
use semantic_data::value::Value;
use semantic_rpc::RpcClientError;

use crate::{
    layout::{
        Entity, EntityFormat, MountConfig, Snapshot, collision_name, escape_name, hash_prefix,
        object_string,
    },
    runtime::{
        Event, RuntimeBridge, UploadSession, batch_payload, entity_payload, link_operation,
        unlink_operation,
    },
};

const TTL: Duration = Duration::from_secs(1);
const ROOT_INODE: u64 = 1;
const EACCES: Errno = Errno::EACCES;
const EBADF: Errno = Errno::EBADF;
const EBUSY: Errno = Errno::EBUSY;
const EINVAL: Errno = Errno::EINVAL;
const EIO: Errno = Errno::EIO;
const ENOENT: Errno = Errno::ENOENT;
const ENOTDIR: Errno = Errno::ENOTDIR;
const EROFS: Errno = Errno::EROFS;

pub(crate) fn mount_options(config: &MountConfig) -> Config {
    let mut options = vec![MountOption::FSName("semantic".to_string())];
    if config.allow_other {
        options.push(MountOption::AutoUnmount);
    }
    if config.read_only {
        options.push(MountOption::RO);
    }
    let mut fuse_config = Config::default();
    fuse_config.mount_options = options;
    fuse_config.acl = if config.allow_other {
        SessionACL::All
    } else {
        SessionACL::Owner
    };
    fuse_config
}

#[derive(Clone, Debug)]
enum NodeKind {
    Directory { entity_id: Option<String> },
    Metadata { entity_id: String, content: Vec<u8> },
    RawFile { entity_id: String },
    Pending,
    Failed(String),
}

#[derive(Clone, Debug)]
struct Node {
    parent: u64,
    name: String,
    kind: NodeKind,
    size: u64,
    children: BTreeMap<String, u64>,
}

impl Node {
    fn file_type(&self) -> FileType {
        match self.kind {
            NodeKind::Directory { .. } => FileType::Directory,
            _ => FileType::RegularFile,
        }
    }
}

#[derive(Debug, Default)]
struct SequentialWrite {
    expected_offset: u64,
    failed: bool,
}

impl SequentialWrite {
    fn accept(&mut self, offset: i64, length: usize) -> std::result::Result<(), ()> {
        if self.failed || offset < 0 || offset as u64 != self.expected_offset {
            self.failed = true;
            return Err(());
        }
        self.expected_offset = self.expected_offset.saturating_add(length as u64);
        Ok(())
    }
}

enum WriteHandle {
    Upload {
        inode: u64,
        filename: String,
        parent: Option<String>,
        declared_size: Option<u64>,
        sequence: SequentialWrite,
        session: Option<UploadSession>,
        finished: bool,
    },
    Entity {
        inode: u64,
        entity_id: Option<String>,
        filename: String,
        parent: Option<String>,
        format: EntityFormat,
        sequence: SequentialWrite,
        buffer: Vec<u8>,
        finished: bool,
    },
}

pub struct SemanticFilesystem {
    state: Mutex<FilesystemState>,
}

struct FilesystemState {
    bridge: RuntimeBridge,
    config: MountConfig,
    nodes: HashMap<u64, Node>,
    handles: HashMap<u64, WriteHandle>,
    next_inode: u64,
    next_handle: u64,
}

impl SemanticFilesystem {
    pub(crate) fn from_snapshot(
        bridge: RuntimeBridge,
        snapshot: Snapshot,
        config: MountConfig,
    ) -> Self {
        Self {
            state: Mutex::new(FilesystemState::from_snapshot(bridge, snapshot, config)),
        }
    }
}

impl FilesystemState {
    fn from_snapshot(bridge: RuntimeBridge, snapshot: Snapshot, config: MountConfig) -> Self {
        let mut filesystem = Self {
            bridge,
            config,
            nodes: HashMap::new(),
            handles: HashMap::new(),
            next_inode: ROOT_INODE + 1,
            next_handle: 1,
        };
        filesystem.nodes.insert(
            ROOT_INODE,
            Node {
                parent: ROOT_INODE,
                name: String::new(),
                kind: NodeKind::Directory { entity_id: None },
                size: 0,
                children: BTreeMap::new(),
            },
        );
        filesystem.build_layout(snapshot);
        filesystem
    }

    fn build_layout(&mut self, snapshot: Snapshot) {
        let entities_root = self.add_directory(ROOT_INODE, "entities", None);
        let tree_root = self.add_directory(ROOT_INODE, "tree", None);
        let files_root = self.add_directory(ROOT_INODE, "files", None);
        let blobs_root = self.add_directory(ROOT_INODE, "blobs", None);

        let mut entity_prefixes = HashMap::new();
        let mut file_prefixes = HashMap::new();
        let mut blob_prefixes = HashMap::new();
        for entity in snapshot.entities.values() {
            let prefix = hash_prefix(&entity.id);
            let prefix_ino = *entity_prefixes
                .entry(prefix.clone())
                .or_insert_with(|| self.add_directory(entities_root, &prefix, None));
            if let Ok(content) = self.config.format.serialize(&entity.object) {
                self.add_metadata(
                    prefix_ino,
                    format!(
                        "{}.{}",
                        escape_name(&entity.id),
                        self.config.format.extension()
                    ),
                    &entity.id,
                    content,
                );
            }
            if entity.is_file() {
                let prefix_ino = *file_prefixes
                    .entry(prefix.clone())
                    .or_insert_with(|| self.add_directory(files_root, &prefix, None));
                let entity_dir = self.add_directory(prefix_ino, &escape_name(&entity.id), None);
                self.add_raw(
                    entity_dir,
                    escape_name(entity.filename().unwrap_or("content")),
                    entity,
                );
                if let Some(hash) = entity.hash() {
                    let blob_prefix = hash.get(..2).unwrap_or(hash).to_ascii_lowercase();
                    let blob_prefix_ino = *blob_prefixes
                        .entry(blob_prefix.clone())
                        .or_insert_with(|| self.add_directory(blobs_root, &blob_prefix, None));
                    if !self.child_exists(blob_prefix_ino, hash) {
                        self.add_raw(blob_prefix_ino, hash.to_string(), entity);
                    }
                }
            }
        }

        let child_directories = snapshot
            .links
            .iter()
            .filter_map(|link| {
                snapshot
                    .entities
                    .get(&link.child)
                    .filter(|entity| entity.is_directory())
                    .map(|_| link.child.clone())
            })
            .collect::<BTreeSet<_>>();
        let mut children = BTreeMap::<String, Vec<String>>::new();
        for link in &snapshot.links {
            children
                .entry(link.parent.clone())
                .or_default()
                .push(link.child.clone());
        }
        for values in children.values_mut() {
            values.sort();
            values.dedup();
        }
        let mut roots = snapshot
            .entities
            .values()
            .filter(|entity| entity.is_directory() && !child_directories.contains(&entity.id))
            .map(|entity| entity.id.clone())
            .collect::<BTreeSet<_>>();
        let mut reachable = roots.clone();
        let mut pending = roots.iter().cloned().collect::<Vec<_>>();
        while let Some(parent) = pending.pop() {
            for child in children.get(&parent).into_iter().flatten() {
                if snapshot
                    .entities
                    .get(child)
                    .is_some_and(Entity::is_directory)
                    && reachable.insert(child.clone())
                {
                    pending.push(child.clone());
                }
            }
        }
        roots.extend(
            snapshot
                .entities
                .values()
                .filter(|entity| entity.is_directory() && !reachable.contains(&entity.id))
                .map(|entity| entity.id.clone()),
        );
        for id in roots {
            self.add_tree_occurrence(
                tree_root,
                &id,
                &snapshot.entities,
                &children,
                &mut BTreeSet::new(),
            );
        }
    }

    fn add_tree_occurrence(
        &mut self,
        parent: u64,
        id: &str,
        entities: &BTreeMap<String, Entity>,
        children: &BTreeMap<String, Vec<String>>,
        path: &mut BTreeSet<String>,
    ) {
        let Some(entity) = entities.get(id) else {
            return;
        };
        if entity.is_directory() {
            if !path.insert(id.to_string()) {
                return;
            }
            let name = self.available_name(parent, escape_name(entity.title()), id);
            let directory = self.add_directory(parent, &name, Some(id.to_string()));
            if let Some(child_ids) = children.get(id) {
                for child in child_ids {
                    self.add_tree_occurrence(directory, child, entities, children, path);
                }
            }
            path.remove(id);
        } else if entity.is_file() {
            let filename = escape_name(entity.filename().unwrap_or("content"));
            let name = self.available_name(
                parent,
                format!("{}_{}", escape_name(&entity.id), filename),
                id,
            );
            self.add_raw(parent, name, entity);
        } else if let Ok(content) = self.config.format.serialize(&entity.object) {
            let name = self.available_name(
                parent,
                format!(
                    "{}.{}",
                    escape_name(&entity.id),
                    self.config.format.extension()
                ),
                id,
            );
            self.add_metadata(parent, name, &entity.id, content);
        }
    }

    fn add_directory(&mut self, parent: u64, name: &str, entity_id: Option<String>) -> u64 {
        self.add_node(
            parent,
            name.to_string(),
            NodeKind::Directory { entity_id },
            0,
        )
    }

    fn add_metadata(&mut self, parent: u64, name: String, id: &str, content: Vec<u8>) -> u64 {
        let size = content.len() as u64;
        self.add_node(
            parent,
            name,
            NodeKind::Metadata {
                entity_id: id.to_string(),
                content,
            },
            size,
        )
    }

    fn add_raw(&mut self, parent: u64, name: String, entity: &Entity) -> u64 {
        self.add_node(
            parent,
            name,
            NodeKind::RawFile {
                entity_id: entity.id.clone(),
            },
            entity.byte_size(),
        )
    }

    fn add_node(&mut self, parent: u64, name: String, kind: NodeKind, size: u64) -> u64 {
        let ino = self.next_inode;
        self.next_inode += 1;
        self.nodes.insert(
            ino,
            Node {
                parent,
                name: name.clone(),
                kind,
                size,
                children: BTreeMap::new(),
            },
        );
        self.nodes
            .get_mut(&parent)
            .expect("virtual parent must exist")
            .children
            .insert(name, ino);
        ino
    }

    fn child_exists(&self, parent: u64, name: &str) -> bool {
        self.nodes
            .get(&parent)
            .is_some_and(|node| node.children.contains_key(name))
    }

    fn available_name(&self, parent: u64, name: String, id: &str) -> String {
        if self.child_exists(parent, &name) {
            collision_name(&name, id)
        } else {
            name
        }
    }

    fn attr(&self, ino: u64) -> Option<FileAttr> {
        let node = self.nodes.get(&ino)?;
        let now = SystemTime::UNIX_EPOCH;
        Some(FileAttr {
            ino: INodeNo(ino),
            size: node.size,
            blocks: node.size.div_ceil(512),
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: node.file_type(),
            perm: if matches!(node.kind, NodeKind::Directory { .. }) {
                0o755
            } else if self.config.read_only {
                0o444
            } else {
                0o644
            },
            nlink: if matches!(node.kind, NodeKind::Directory { .. }) {
                2
            } else {
                1
            },
            uid: unsafe { libc::geteuid() },
            gid: unsafe { libc::getegid() },
            rdev: 0,
            blksize: 4096,
            flags: 0,
        })
    }

    fn directory_entity_id(&self, ino: u64) -> Option<String> {
        match &self.nodes.get(&ino)?.kind {
            NodeKind::Directory { entity_id } => entity_id.clone(),
            _ => None,
        }
    }

    fn drain_events(&mut self) {
        for event in self.bridge.drain_events() {
            match event {
                Event::UploadFinished { inode, result } => {
                    let Some(node) = self.nodes.get_mut(&inode) else {
                        continue;
                    };
                    match result {
                        Ok(response) => {
                            let filename = object_string(
                                &response.object,
                                &["filename", semantic_data::filestore::ATTR_FILE_FILENAME],
                            )
                            .unwrap_or(&node.name);
                            let new_name =
                                format!("{}_{}", escape_name(&response.id), escape_name(filename));
                            let parent = node.parent;
                            let old_name = std::mem::replace(&mut node.name, new_name.clone());
                            node.size = response
                                .object
                                .get("byte_size")
                                .and_then(|value| match value {
                                    Value::U64(value) => Some(*value),
                                    _ => None,
                                })
                                .unwrap_or(node.size);
                            node.kind = NodeKind::RawFile {
                                entity_id: response.id,
                            };
                            if let Some(parent) = self.nodes.get_mut(&parent) {
                                parent.children.remove(&old_name);
                                parent.children.insert(new_name, inode);
                            }
                        }
                        Err(error) => node.kind = NodeKind::Failed(error),
                    }
                }
            }
        }
    }

    fn allocate_handle(&mut self, handle: WriteHandle) -> u64 {
        let fh = self.next_handle;
        self.next_handle += 1;
        self.handles.insert(fh, handle);
        fh
    }

    fn commit(&mut self, fh: u64) -> std::result::Result<(), Errno> {
        let Some(mut handle) = self.handles.remove(&fh) else {
            return Err(EBADF);
        };
        let result = match &mut handle {
            WriteHandle::Upload {
                inode,
                filename,
                parent,
                declared_size,
                sequence,
                session,
                finished,
                ..
            } => {
                if sequence.failed {
                    Err(EINVAL)
                } else if *finished {
                    Ok(())
                } else {
                    if session.is_none() {
                        *session = Some(self.bridge.begin_upload(
                            *inode,
                            filename.clone(),
                            parent.clone(),
                            *declared_size,
                            self.config.scope_id.clone(),
                        ));
                    }
                    let session = session.as_mut().expect("upload session initialized");
                    session.sender.take();
                    match session.completion.recv() {
                        Ok(Ok(_)) => {
                            *finished = true;
                            Ok(())
                        }
                        Ok(Err(_)) | Err(_) => Err(EIO),
                    }
                }
            }
            WriteHandle::Entity {
                inode,
                entity_id,
                filename,
                parent,
                format,
                sequence,
                buffer,
                finished,
                ..
            } => {
                if sequence.failed {
                    Err(EINVAL)
                } else if *finished {
                    Ok(())
                } else {
                    let parsed = format.parse(buffer).ok().and_then(|object| {
                        let id = entity_id.clone().or_else(|| {
                            object_string(&object, &["id", "semantic:id"]).map(str::to_string)
                        })?;
                        Some((id, object))
                    });
                    let Some((id, mut object)) = parsed else {
                        let Some(parent) = parent.clone() else {
                            return Err(EINVAL);
                        };
                        let mut upload = self.bridge.begin_upload(
                            *inode,
                            filename.clone(),
                            Some(parent),
                            Some(buffer.len() as u64),
                            self.config.scope_id.clone(),
                        );
                        let mut sender = upload.sender.take().ok_or(EIO)?;
                        futures::executor::block_on(
                            sender.send(Ok(Bytes::copy_from_slice(buffer))),
                        )
                        .map_err(|_| EIO)?;
                        drop(sender);
                        match upload.completion.recv() {
                            Ok(Ok(_)) => {
                                *finished = true;
                                return Ok(());
                            }
                            Ok(Err(_)) | Err(_) => return Err(EIO),
                        }
                    };
                    object.insert("id", Value::String(id.clone()));
                    self.bridge
                        .invoke(
                            "semantic.db.insert",
                            entity_payload(&self.config, &id, object.clone()),
                        )
                        .map_err(|_| EIO)?;
                    if let Some(parent) = parent {
                        self.bridge
                            .invoke(
                                "semantic.db.batch",
                                batch_payload(&self.config, vec![link_operation(parent, &id)]),
                            )
                            .map_err(|_| EIO)?;
                    }
                    if let Some(node) = self.nodes.get_mut(inode) {
                        node.size = buffer.len() as u64;
                        node.kind = NodeKind::Metadata {
                            entity_id: id,
                            content: buffer.clone(),
                        };
                    }
                    *finished = true;
                    Ok(())
                }
            }
        };
        self.handles.insert(fh, handle);
        result
    }

    fn abort_upload(handle: &mut WriteHandle, message: &str) {
        let WriteHandle::Upload { session, .. } = handle else {
            return;
        };
        if let Some(session) = session {
            if let Some(mut sender) = session.sender.take() {
                let _ = sender.try_send(Err(RpcClientError::Transport(message.to_string())));
            }
        }
    }
}

impl FilesystemState {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        self.drain_events();
        let Some(name) = name.to_str() else {
            reply.error(EINVAL);
            return;
        };
        let Some(ino) = self
            .nodes
            .get(&parent)
            .and_then(|node| node.children.get(name))
            .copied()
        else {
            reply.error(ENOENT);
            return;
        };
        reply.entry(
            &TTL,
            &self.attr(ino).expect("child inode must exist"),
            Generation(0),
        );
    }

    fn getattr(&mut self, _req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        self.drain_events();
        match self.attr(ino) {
            Some(attr) => reply.attr(&TTL, &attr),
            None => reply.error(ENOENT),
        }
    }

    fn setattr(
        &mut self,
        _req: &Request,
        ino: u64,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        if self.config.read_only {
            reply.error(EROFS);
            return;
        }
        if let Some(size) = size {
            if let Some(node) = self.nodes.get_mut(&ino) {
                node.size = size;
            }
            if let Some(WriteHandle::Upload { declared_size, .. }) =
                fh.and_then(|fh| self.handles.get_mut(&fh))
            {
                *declared_size = Some(size);
            }
        }
        match self.attr(ino) {
            Some(attr) => reply.attr(&TTL, &attr),
            None => reply.error(ENOENT),
        }
    }

    fn open(&mut self, _req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
        let Some(node) = self.nodes.get(&ino) else {
            reply.error(ENOENT);
            return;
        };
        if matches!(node.kind, NodeKind::Directory { .. }) {
            reply.error(EINVAL);
            return;
        }
        let writing = flags & libc::O_ACCMODE != libc::O_RDONLY;
        if !writing {
            reply.opened(FileHandle(0), FopenFlags::FOPEN_DIRECT_IO);
            return;
        }
        if self.config.read_only {
            reply.error(EROFS);
            return;
        }
        let NodeKind::Metadata { entity_id, .. } = &node.kind else {
            reply.error(EROFS);
            return;
        };
        if self.handles.values().any(|handle| match handle {
            WriteHandle::Upload { inode, .. } | WriteHandle::Entity { inode, .. } => *inode == ino,
        }) {
            reply.error(EBUSY);
            return;
        }
        let format = self.config.format;
        let fh = self.allocate_handle(WriteHandle::Entity {
            inode: ino,
            entity_id: Some(entity_id.clone()),
            filename: node.name.clone(),
            parent: None,
            format,
            sequence: SequentialWrite::default(),
            buffer: Vec::new(),
            finished: false,
        });
        reply.opened(FileHandle(fh), FopenFlags::FOPEN_DIRECT_IO);
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        self.drain_events();
        if offset < 0 {
            reply.error(EINVAL);
            return;
        }
        let Some(node) = self.nodes.get(&ino) else {
            reply.error(ENOENT);
            return;
        };
        match &node.kind {
            NodeKind::Metadata { content, .. } => {
                let start = (offset as usize).min(content.len());
                let end = start.saturating_add(size as usize).min(content.len());
                reply.data(&content[start..end]);
            }
            NodeKind::RawFile { entity_id } => {
                match self.bridge.read(
                    entity_id.clone(),
                    self.config.scope_id.clone(),
                    offset as u64,
                    size,
                ) {
                    Ok(bytes) => reply.data(&bytes),
                    Err(_) => reply.error(EIO),
                }
            }
            NodeKind::Failed(error) => {
                tracing::error!(inode = ino, error, "FUSE upload failed");
                reply.error(EIO);
            }
            NodeKind::Pending => reply.error(EBUSY),
            NodeKind::Directory { .. } => reply.error(EINVAL),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        self.drain_events();
        let Some(node) = self.nodes.get(&ino) else {
            reply.error(ENOENT);
            return;
        };
        if !matches!(node.kind, NodeKind::Directory { .. }) {
            reply.error(ENOTDIR);
            return;
        }
        let mut entries = vec![
            (ino, FileType::Directory, ".".to_string()),
            (node.parent, FileType::Directory, "..".to_string()),
        ];
        entries.extend(node.children.iter().filter_map(|(name, child)| {
            self.nodes
                .get(child)
                .map(|node| (*child, node.file_type(), name.clone()))
        }));
        for (index, (child, kind, name)) in
            entries.into_iter().enumerate().skip(offset.max(0) as usize)
        {
            if reply.add(INodeNo(child), (index + 1) as u64, kind, name) {
                break;
            }
        }
        reply.ok();
    }

    fn create(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        if self.config.read_only {
            reply.error(EROFS);
            return;
        }
        let Some(name) = name.to_str() else {
            reply.error(EINVAL);
            return;
        };
        if self.child_exists(parent, name) {
            reply.error(Errno::EEXIST);
            return;
        }
        let tree_parent = self.directory_entity_id(parent);
        let entities_parent = self
            .nodes
            .get(&parent)
            .and_then(|node| self.nodes.get(&node.parent))
            .is_some_and(|node| node.name == "entities");
        if tree_parent.is_none() && !entities_parent {
            reply.error(EACCES);
            return;
        }
        let document_format = if name.ends_with(".json") {
            Some(EntityFormat::Json)
        } else if name.ends_with(".yaml") || name.ends_with(".yml") {
            Some(EntityFormat::Yaml)
        } else {
            None
        };
        let inode = self.add_node(parent, name.to_string(), NodeKind::Pending, 0);
        let handle = if let Some(format) = document_format {
            WriteHandle::Entity {
                inode,
                entity_id: None,
                filename: name.to_string(),
                parent: tree_parent,
                format,
                sequence: SequentialWrite::default(),
                buffer: Vec::new(),
                finished: false,
            }
        } else if let Some(parent) = tree_parent {
            WriteHandle::Upload {
                inode,
                filename: name.to_string(),
                parent: Some(parent),
                declared_size: None,
                sequence: SequentialWrite::default(),
                session: None,
                finished: false,
            }
        } else {
            self.nodes.remove(&inode);
            if let Some(parent) = self.nodes.get_mut(&parent) {
                parent.children.remove(name);
            }
            reply.error(EINVAL);
            return;
        };
        let fh = self.allocate_handle(handle);
        reply.created(
            &TTL,
            &self.attr(inode).expect("created inode must exist"),
            Generation(0),
            FileHandle(fh),
            FopenFlags::FOPEN_DIRECT_IO,
        );
    }

    fn write(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        let Some(handle) = self.handles.get_mut(&fh) else {
            reply.error(EBADF);
            return;
        };
        let sequence = match handle {
            WriteHandle::Upload { sequence, .. } | WriteHandle::Entity { sequence, .. } => sequence,
        };
        if sequence.accept(offset, data.len()).is_err() {
            Self::abort_upload(handle, "out-of-order FUSE write");
            reply.error(EINVAL);
            return;
        }
        match handle {
            WriteHandle::Upload {
                inode,
                filename,
                parent,
                declared_size,
                session,
                ..
            } => {
                if session.is_none() {
                    *session = Some(self.bridge.begin_upload(
                        *inode,
                        filename.clone(),
                        parent.clone(),
                        *declared_size,
                        self.config.scope_id.clone(),
                    ));
                }
                let sender = session
                    .as_mut()
                    .and_then(|session| session.sender.as_mut())
                    .expect("active upload must have sender");
                if futures::executor::block_on(sender.send(Ok(Bytes::copy_from_slice(data))))
                    .is_err()
                {
                    reply.error(EIO);
                    return;
                }
            }
            WriteHandle::Entity { buffer, .. } => buffer.extend_from_slice(data),
        }
        reply.written(data.len() as u32);
    }

    fn flush(&mut self, _req: &Request, _ino: u64, fh: u64, _lock_owner: u64, reply: ReplyEmpty) {
        match self.commit(fh) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(error),
        }
    }

    fn fsync(&mut self, _req: &Request, _ino: u64, fh: u64, _datasync: bool, reply: ReplyEmpty) {
        match self.commit(fh) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(error),
        }
    }

    fn release(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        let result = self.commit(fh);
        self.handles.remove(&fh);
        match result {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(error),
        }
    }

    fn rename(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        flags: u32,
        reply: ReplyEmpty,
    ) {
        if self.config.read_only {
            reply.error(EROFS);
            return;
        }
        if flags != 0 {
            reply.error(EINVAL);
            return;
        }
        let (Some(name), Some(newname)) = (name.to_str(), newname.to_str()) else {
            reply.error(EINVAL);
            return;
        };
        if name != newname {
            reply.error(EINVAL);
            return;
        }
        if self.child_exists(newparent, newname) && parent != newparent {
            reply.error(Errno::EEXIST);
            return;
        }
        let Some(inode) = self
            .nodes
            .get(&parent)
            .and_then(|node| node.children.get(name))
            .copied()
        else {
            reply.error(ENOENT);
            return;
        };
        let (Some(old_parent_id), Some(new_parent_id)) = (
            self.directory_entity_id(parent),
            self.directory_entity_id(newparent),
        ) else {
            reply.error(EROFS);
            return;
        };
        let entity_id = match &self.nodes.get(&inode).expect("child exists").kind {
            NodeKind::Directory {
                entity_id: Some(id),
            }
            | NodeKind::Metadata { entity_id: id, .. }
            | NodeKind::RawFile { entity_id: id } => id.clone(),
            _ => {
                reply.error(EBUSY);
                return;
            }
        };
        let operations = vec![
            unlink_operation(&old_parent_id, &entity_id),
            link_operation(&new_parent_id, &entity_id),
        ];
        if self
            .bridge
            .invoke("semantic.db.batch", batch_payload(&self.config, operations))
            .is_err()
        {
            reply.error(EIO);
            return;
        }
        self.nodes
            .get_mut(&parent)
            .expect("parent exists")
            .children
            .remove(name);
        self.nodes
            .get_mut(&newparent)
            .expect("new parent exists")
            .children
            .insert(newname.to_string(), inode);
        let node = self.nodes.get_mut(&inode).expect("child exists");
        node.parent = newparent;
        node.name = newname.to_string();
        reply.ok();
    }
}

impl Filesystem for SemanticFilesystem {
    fn lookup(&self, req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        self.state
            .lock()
            .expect("FUSE state poisoned")
            .lookup(req, parent.0, name, reply);
    }

    fn getattr(&self, req: &Request, ino: INodeNo, fh: Option<FileHandle>, reply: ReplyAttr) {
        self.state.lock().expect("FUSE state poisoned").getattr(
            req,
            ino.0,
            fh.map(|handle| handle.0),
            reply,
        );
    }

    fn setattr(
        &self,
        req: &Request,
        ino: INodeNo,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        ctime: Option<SystemTime>,
        fh: Option<FileHandle>,
        crtime: Option<SystemTime>,
        chgtime: Option<SystemTime>,
        bkuptime: Option<SystemTime>,
        flags: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        self.state.lock().expect("FUSE state poisoned").setattr(
            req,
            ino.0,
            mode,
            uid,
            gid,
            size,
            atime,
            mtime,
            ctime,
            fh.map(|handle| handle.0),
            crtime,
            chgtime,
            bkuptime,
            flags.map(|flags| flags.bits()),
            reply,
        );
    }

    fn open(&self, req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        self.state
            .lock()
            .expect("FUSE state poisoned")
            .open(req, ino.0, flags.0, reply);
    }

    fn read(
        &self,
        req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        flags: OpenFlags,
        lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        self.state.lock().expect("FUSE state poisoned").read(
            req,
            ino.0,
            fh.0,
            offset.min(i64::MAX as u64) as i64,
            size,
            flags.0,
            lock_owner.map(|owner| owner.0),
            reply,
        );
    }

    fn readdir(
        &self,
        req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        reply: ReplyDirectory,
    ) {
        self.state.lock().expect("FUSE state poisoned").readdir(
            req,
            ino.0,
            fh.0,
            offset.min(i64::MAX as u64) as i64,
            reply,
        );
    }

    fn create(
        &self,
        req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        self.state
            .lock()
            .expect("FUSE state poisoned")
            .create(req, parent.0, name, mode, umask, flags, reply);
    }

    fn write(
        &self,
        req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        data: &[u8],
        write_flags: WriteFlags,
        flags: OpenFlags,
        lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        self.state.lock().expect("FUSE state poisoned").write(
            req,
            ino.0,
            fh.0,
            offset.min(i64::MAX as u64) as i64,
            data,
            write_flags.bits(),
            flags.0,
            lock_owner.map(|owner| owner.0),
            reply,
        );
    }

    fn flush(
        &self,
        req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        lock_owner: LockOwner,
        reply: ReplyEmpty,
    ) {
        self.state.lock().expect("FUSE state poisoned").flush(
            req,
            ino.0,
            fh.0,
            lock_owner.0,
            reply,
        );
    }

    fn fsync(
        &self,
        req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        datasync: bool,
        reply: ReplyEmpty,
    ) {
        self.state
            .lock()
            .expect("FUSE state poisoned")
            .fsync(req, ino.0, fh.0, datasync, reply);
    }

    fn release(
        &self,
        req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        flags: OpenFlags,
        lock_owner: Option<LockOwner>,
        flush: bool,
        reply: ReplyEmpty,
    ) {
        self.state.lock().expect("FUSE state poisoned").release(
            req,
            ino.0,
            fh.0,
            flags.0,
            lock_owner.map(|owner| owner.0),
            flush,
            reply,
        );
    }

    fn rename(
        &self,
        req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        self.state.lock().expect("FUSE state poisoned").rename(
            req,
            parent.0,
            name,
            newparent.0,
            newname,
            flags.bits(),
            reply,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_data::{
        bundles::directory::DIRECTORY_CLASS_ID,
        filestore::{ATTR_FILE_BYTE_SIZE, ATTR_FILE_FILENAME, FILE_CLASS_ID},
        value::Object,
    };
    use semantic_rpc::{RpcClient, RpcClientDyn, RpcClientError};

    struct MockClient;

    impl RpcClientDyn for MockClient {
        fn invoke_value(
            &self,
            _command: String,
            _payload: Value,
        ) -> semantic_rpc::client::RpcClientFuture<std::result::Result<Value, RpcClientError>>
        {
            Box::pin(async { Ok(Value::Null) })
        }
    }

    #[test]
    fn sequential_writes_accept_contiguous_offsets() {
        let mut write = SequentialWrite::default();
        assert_eq!(write.accept(0, 4), Ok(()));
        assert_eq!(write.accept(4, 2), Ok(()));
        assert_eq!(write.expected_offset, 6);
    }

    #[test]
    fn sequential_writes_fail_closed_after_out_of_order_offset() {
        let mut write = SequentialWrite::default();
        assert_eq!(write.accept(0, 4), Ok(()));
        assert_eq!(write.accept(3, 1), Err(()));
        assert_eq!(write.accept(4, 1), Err(()));
    }

    #[test]
    fn selected_entity_format_controls_tree_suffix() {
        assert_eq!(EntityFormat::Json.extension(), "json");
        assert_eq!(EntityFormat::Yaml.extension(), "yaml");
    }

    #[test]
    fn directory_type_constant_matches_layout_expectation() {
        assert_eq!(DIRECTORY_CLASS_ID, "semantic:base:directory");
    }

    #[test]
    fn builds_the_four_views_and_human_named_tree_directories() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let bridge = RuntimeBridge::new(RpcClient::new(MockClient), runtime.handle().clone());
        let mut directory = Object::new();
        directory.insert("id", Value::String("dir-1".to_string()));
        directory.insert("type", Value::String(DIRECTORY_CLASS_ID.to_string()));
        directory.insert("title", Value::String("Projects".to_string()));
        let mut note = Object::new();
        note.insert("id", Value::String("note-1".to_string()));
        note.insert("type", Value::String("semantic:note".to_string()));
        let mut file = Object::new();
        file.insert("id", Value::String("file-1".to_string()));
        file.insert("type", Value::String(FILE_CLASS_ID.to_string()));
        file.insert(ATTR_FILE_FILENAME, Value::String("report.pdf".to_string()));
        file.insert(ATTR_FILE_BYTE_SIZE, Value::U64(12));
        let entities = [("dir-1", directory), ("note-1", note), ("file-1", file)]
            .into_iter()
            .map(|(id, object)| {
                (
                    id.to_string(),
                    Entity {
                        id: id.to_string(),
                        object,
                    },
                )
            })
            .collect();
        let snapshot = Snapshot {
            entities,
            links: vec![
                crate::layout::DirectoryLink {
                    parent: "dir-1".to_string(),
                    child: "note-1".to_string(),
                },
                crate::layout::DirectoryLink {
                    parent: "dir-1".to_string(),
                    child: "file-1".to_string(),
                },
            ],
        };
        let filesystem =
            SemanticFilesystem::from_snapshot(bridge, snapshot, MountConfig::default());
        let state = filesystem.state.lock().unwrap();
        let root = state.nodes.get(&ROOT_INODE).unwrap();
        assert_eq!(
            root.children.keys().cloned().collect::<Vec<_>>(),
            vec!["blobs", "entities", "files", "tree"]
        );
        let tree = root.children["tree"];
        let projects = state.nodes[&tree].children["Projects"];
        let child_names = state.nodes[&projects]
            .children
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(child_names, vec!["file-1_report.pdf", "note-1.json"]);
    }
}
