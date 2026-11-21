// SPDX-License-Identifier: LGPL-2.1-or-later
//! Writing a document's objects into a schema-v1 database and reading them
//! back.
//!
//! Objects map to rows column by column; properties use the explicit codec;
//! dependencies are rows of `deps`; payloads above [`CHUNK_BYTES`] are split
//! into `blobs` (SQLite caps one blob at about 1 GB, and smaller chunks keep
//! memory use flat while reading). Objects are written in ID order, so the
//! same document always produces the same rows.

use crate::codec::{self, decode_props, encode_props, state_from_i64, state_to_i64};
use crate::{FormatError, sql};
use pxp_doc::{AppId, Dep, Layer, NewObject, Object, ObjectId, ObjectKind, Snapshot};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::BTreeMap;

/// Payloads larger than this are stored as ordered chunks in `blobs`.
pub const CHUNK_BYTES: usize = 8 * 1024 * 1024;

/// Inserts every object of `snap`. Returns the number of objects written.
pub(crate) fn write_objects(tx: &Transaction<'_>, snap: &Snapshot) -> Result<usize, FormatError> {
    let mut obj_stmt = tx
        .prepare(
            "INSERT INTO objects (id, kind, kind_ver, owner_app, parent, name, layer, state, \
             diagnostic, props, payload, payload_chunks) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )
        .map_err(sql("prepare object insert"))?;
    let mut dep_stmt = tx
        .prepare("INSERT INTO deps (from_id, to_id, role) VALUES (?1, ?2, ?3)")
        .map_err(sql("prepare dependency insert"))?;
    let mut blob_stmt = tx
        .prepare("INSERT INTO blobs (blob_id, seq, data) VALUES (?1, ?2, ?3)")
        .map_err(sql("prepare blob insert"))?;
    let mut count = 0;
    for o in snap.objects() {
        let id = o.id().to_bytes();
        let chunked = o.payload.len() > CHUNK_BYTES;
        let chunks: i64 = if chunked {
            i64::try_from(o.payload.chunks(CHUNK_BYTES).len()).unwrap_or(i64::MAX)
        } else {
            0
        };
        let inline_payload: &[u8] = if chunked { &[] } else { &o.payload };
        obj_stmt
            .execute(params![
                id,
                o.kind.as_str(),
                i64::from(o.kind_ver),
                o.owner_app.as_str(),
                o.parent.map(ObjectId::to_bytes),
                o.name,
                i64::from(o.layer.get()),
                state_to_i64(o.state),
                o.diagnostic,
                encode_props(&o.props),
                inline_payload,
                chunks,
            ])
            .map_err(sql("write object"))?;
        for d in &o.deps {
            dep_stmt
                .execute(params![id, d.on.to_bytes(), d.role])
                .map_err(sql("write dependency"))?;
        }
        if chunked {
            for (seq, part) in o.payload.chunks(CHUNK_BYTES).enumerate() {
                let seq = i64::try_from(seq).unwrap_or(i64::MAX);
                blob_stmt
                    .execute(params![id, seq, part])
                    .map_err(sql("write payload chunk"))?;
            }
        }
        count += 1;
    }
    Ok(count)
}

fn id_from_blob(bytes: &[u8], what: &str) -> Result<ObjectId, FormatError> {
    let arr: [u8; 16] = bytes
        .try_into()
        .map_err(|_| FormatError::Corrupt(format!("{what} is {} bytes, not 16", bytes.len())))?;
    Ok(ObjectId::from_bytes(arr))
}

/// Reads every object. Rows are validated individually here; the document
/// validates the graph as a whole when it is built.
pub(crate) fn read_objects(conn: &Connection) -> Result<BTreeMap<ObjectId, Object>, FormatError> {
    let mut deps: BTreeMap<ObjectId, Vec<Dep>> = BTreeMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT from_id, to_id, role FROM deps")
            .map_err(sql("read dependencies"))?;
        let mut rows = stmt.query([]).map_err(sql("read dependencies"))?;
        while let Some(row) = rows.next().map_err(sql("read dependencies"))? {
            let from: Vec<u8> = row.get(0).map_err(sql("read dependency"))?;
            let to: Vec<u8> = row.get(1).map_err(sql("read dependency"))?;
            let role: String = row.get(2).map_err(sql("read dependency"))?;
            deps.entry(id_from_blob(&from, "dependency source")?)
                .or_default()
                .push(Dep::new(id_from_blob(&to, "dependency target")?, role));
        }
    }
    let mut out = BTreeMap::new();
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, kind_ver, owner_app, parent, name, layer, state, diagnostic, props, \
             payload, payload_chunks FROM objects",
        )
        .map_err(sql("read objects"))?;
    let mut rows = stmt.query([]).map_err(sql("read objects"))?;
    while let Some(row) = rows.next().map_err(sql("read objects"))? {
        let get_err = sql("read object column");
        let raw_id: Vec<u8> = row.get(0).map_err(get_err)?;
        let id = id_from_blob(&raw_id, "object id")?;
        let ctx = |e: String| FormatError::Corrupt(format!("object {id}: {e}"));
        let kind: String = row.get(1).map_err(sql("read object kind"))?;
        let kind_ver: i64 = row.get(2).map_err(sql("read object version"))?;
        let owner: String = row.get(3).map_err(sql("read object owner"))?;
        let parent: Option<Vec<u8>> = row.get(4).map_err(sql("read object parent"))?;
        let name: Option<String> = row.get(5).map_err(sql("read object name"))?;
        let layer: i64 = row.get(6).map_err(sql("read object layer"))?;
        let state: i64 = row.get(7).map_err(sql("read object state"))?;
        let diagnostic: Option<String> = row.get(8).map_err(sql("read object diagnostic"))?;
        let props: Vec<u8> = row.get(9).map_err(sql("read object properties"))?;
        let payload: Vec<u8> = row.get(10).map_err(sql("read object payload"))?;
        let chunks: i64 = row.get(11).map_err(sql("read object payload"))?;

        let kind = ObjectKind::new(kind).map_err(|e| ctx(e.to_string()))?;
        let owner = AppId::new(owner).map_err(|e| ctx(e.to_string()))?;
        let kind_ver =
            u32::try_from(kind_ver).map_err(|_| ctx(format!("kind version {kind_ver}")))?;
        let layer = u16::try_from(layer)
            .map_err(|_| ctx(format!("layer {layer}")))
            .and_then(|l| Layer::new(l).map_err(|e| ctx(e.to_string())))?;
        let state = state_from_i64(state).map_err(|e| ctx(e.to_string()))?;
        let props = decode_props(&props).map_err(|source| FormatError::Codec {
            id: id.to_string(),
            source,
        })?;
        let payload = if chunks > 0 {
            read_chunks(conn, id, chunks)?
        } else {
            payload
        };

        let mut spec = NewObject::new(kind, owner)
            .kind_ver(kind_ver)
            .layer(layer)
            .state(state)
            .diagnostic(diagnostic)
            .payload(payload);
        if let Some(p) = parent {
            spec = spec.parent(id_from_blob(&p, "parent id")?);
        }
        if let Some(n) = name {
            spec = spec.name(n);
        }
        for d in deps.remove(&id).unwrap_or_default() {
            spec = spec.dep(d.on, d.role);
        }
        for (k, v) in props {
            spec = spec.prop(k, v);
        }
        out.insert(id, Object::restore(id, spec));
    }
    if let Some((from, _)) = deps.into_iter().next() {
        return Err(FormatError::Corrupt(format!(
            "dependency rows for missing object {from}"
        )));
    }
    Ok(out)
}

fn read_chunks(conn: &Connection, id: ObjectId, expected: i64) -> Result<Vec<u8>, FormatError> {
    let mut stmt = conn
        .prepare("SELECT seq, data FROM blobs WHERE blob_id = ?1 ORDER BY seq")
        .map_err(sql("read payload chunks"))?;
    let mut rows = stmt
        .query(params![id.to_bytes()])
        .map_err(sql("read payload chunks"))?;
    let mut out = Vec::new();
    let mut next = 0_i64;
    while let Some(row) = rows.next().map_err(sql("read payload chunk"))? {
        let seq: i64 = row.get(0).map_err(sql("read payload chunk"))?;
        if seq != next {
            return Err(FormatError::Corrupt(format!(
                "object {id}: payload chunk {next} missing"
            )));
        }
        let data: Vec<u8> = row.get(1).map_err(sql("read payload chunk"))?;
        out.extend_from_slice(&data);
        next += 1;
    }
    if next == expected {
        Ok(out)
    } else {
        Err(FormatError::Corrupt(format!(
            "object {id}: {next} of {expected} payload chunks present"
        )))
    }
}

/// Writes one `meta` row.
pub(crate) fn put_meta(tx: &Transaction<'_>, key: &str, value: &[u8]) -> Result<(), FormatError> {
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        params![key, value],
    )
    .map_err(sql("write part settings"))?;
    Ok(())
}

/// Reads one `meta` row, if present.
pub(crate) fn get_meta(conn: &Connection, key: &str) -> Result<Option<Vec<u8>>, FormatError> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
        r.get(0)
    })
    .optional()
    .map_err(sql("read part settings"))
}

/// Encodes one object for the recovery journal (re-exported for clarity).
pub(crate) fn journal_bytes(o: &Object) -> Vec<u8> {
    codec::encode_object(o)
}
