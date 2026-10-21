package com.pokedaisy.app

import org.json.JSONObject
import java.io.File

/**
 * Cloud saves through a synced folder: the saves folder ([SavesLocation.dir]) put inside OneDrive,
 * Dropbox, Google Drive or iCloud Drive, whose own client carries the files between devices. No server,
 * no account of ours. What this adds on top of a plain folder:
 *
 * - [cloudFolders]: the sync clients' folders on this machine, offered as the saves folder.
 * - [PlayMarker]: `<save>.playing` beside the save while a game is open, kept fresh by a heartbeat, so a
 *   second device opening the same game can warn first (two devices writing one save, the sync client
 *   keeps whichever lands last).
 * - [conflictCopies]: the copies a sync client makes when two devices did write it anyway
 *   (`Name-PC.sav`, `Name (conflicted copy ...).sav`, `Name (1).sav`), so the player can pick one.
 *
 * The save file itself is written by the core, in place, as the game saves; [SaveBackups] keeps the
 * copy from before each launch. Plain JVM: the same code runs on Android and the desktop.
 */
object SyncedSaves {

    /** A sync client's folder: [name] for the player ("OneDrive"), [root] the folder it syncs. */
    data class CloudFolder(val name: String, val root: File) {
        /** Where its saves go inside it: the Windows app's (Adventure Pad) folder, so both find one set. */
        val savesDir: File get() = File(root, "AdventurePad/saves")
    }

    /** The sync folders found on this machine, most likely first. [env] / [home] are for tests. */
    fun cloudFolders(
        env: Map<String, String> = System.getenv(),
        home: File = File(System.getProperty("user.home")),
        roots: List<File> = File.listRoots()?.toList().orEmpty(),
    ): List<CloudFolder> = buildList {
        // OneDrive sets these for the signed-in accounts (personal, then work / school).
        for (k in listOf("OneDriveConsumer", "OneDrive", "OneDriveCommercial")) {
            env[k]?.let { add(CloudFolder("OneDrive", File(it))) }
        }
        add(CloudFolder("OneDrive", File(home, "OneDrive")))
        dropboxRoot(env, home)?.let { add(CloudFolder("Dropbox", it)) }
        add(CloudFolder("Dropbox", File(home, "Dropbox")))
        // Google Drive for desktop mounts a drive letter with "My Drive" in it.
        for (r in roots) add(CloudFolder("Google Drive", File(r, "My Drive")))
        add(CloudFolder("Google Drive", File(home, "Google Drive/My Drive")))
        add(CloudFolder("iCloud Drive", File(home, "iCloudDrive")))
        add(CloudFolder("iCloud Drive", File(home, "Library/Mobile Documents/com~apple~CloudDocs")))
    }.filter { it.root.isDirectory }.distinctBy { it.root.absoluteFile.normalize().path.lowercase() }

    /** Dropbox writes where its folder is to info.json ({"personal": {"path": ...}}). */
    private fun dropboxRoot(env: Map<String, String>, home: File): File? {
        val candidates = listOfNotNull(
            env["LOCALAPPDATA"]?.let { File(it, "Dropbox/info.json") },
            env["APPDATA"]?.let { File(it, "Dropbox/info.json") },
            File(home, ".dropbox/info.json"),
        )
        for (f in candidates) {
            if (!f.isFile) continue
            val json = runCatching { JSONObject(f.readText()) }.getOrNull() ?: continue
            for (k in listOf("personal", "business")) {
                json.optJSONObject(k)?.optString("path")?.takeIf { it.isNotBlank() }?.let { return File(it) }
            }
        }
        return null
    }

    /** The sync client [dir] is in, or null for a folder that isn't synced (as far as we can tell). */
    fun cloudFor(dir: File, folders: List<CloudFolder> = cloudFolders()): CloudFolder? {
        val path = dir.absoluteFile.normalize().path.lowercase()
        return folders.firstOrNull { path.startsWith(it.root.absoluteFile.normalize().path.lowercase()) }
    }

    /**
     * Copies a sync client made of [save] when two devices wrote it: in the same folder, the save's base
     * name plus OneDrive's `-<device>` suffix, Dropbox's ` (... conflicted copy ...)` or Google Drive's
     * ` (1)`, with the same extension. Newest first.
     */
    fun conflictCopies(save: File): List<File> {
        val dir = save.parentFile ?: return emptyList()
        val base = save.nameWithoutExtension
        val ext = save.extension
        // OneDrive's suffix is the computer's name, upper case (DESKTOP-4F2K9QX): a lower-case one is
        // another game's save (Emerald.sav / Emerald-Legacy.sav), never a copy.
        val pattern = Regex(
            Regex.escape(base) + """(-[A-Z0-9][A-Z0-9-]{2,}| \(.*conflicted copy.*\)| \(\d+\))\.""" + Regex.escape(ext),
        )
        return dir.listFiles { f -> f.isFile && f != save && pattern.matches(f.name) }
            ?.sortedByDescending { it.lastModified() }
            .orEmpty()
    }

    /**
     * Puts [copy] in [save]'s place, keeping the current save as a backup first ([SaveBackups]'s folder),
     * and removes the copy. For the player's pick between conflicting copies.
     */
    fun useCopy(save: File, copy: File): Boolean = runCatching {
        if (save.isFile) SaveBackups.snapshot(save)
        val tmp = File(save.parentFile, save.name + ".tmp")
        copy.copyTo(tmp, overwrite = true)
        if (!tmp.renameTo(save)) { save.delete(); check(tmp.renameTo(save)) }
        copy.delete()
    }.isSuccess

    /**
     * `<save>.playing`: which device has this save open, and when it last said so. [device] names this
     * one ("Windows PC", the phone's model). Written on start, every [HEARTBEAT_MS] while playing, and
     * removed on close; a marker older than [STALE_MS] is a device that crashed or lost its connection.
     */
    class PlayMarker(private val save: File, private val device: String, private val deviceId: String) {
        val file: File get() = File(save.parentFile, save.name + ".playing")

        /** Another device's fresh marker on this save, or null (none, our own, or stale). */
        fun otherDevice(now: Long = System.currentTimeMillis()): Holder? {
            val f = file
            if (!f.isFile) return null
            val json = runCatching { JSONObject(f.readText()) }.getOrNull() ?: return null
            val id = json.optString("id")
            val beat = json.optLong("heartbeat")
            if (id == deviceId || now - beat > STALE_MS) return null
            return Holder(json.optString("device", "another device"), beat)
        }

        fun touch(now: Long = System.currentTimeMillis()) {
            runCatching {
                val json = JSONObject().put("device", device).put("id", deviceId).put("heartbeat", now)
                val tmp = File(save.parentFile, save.name + ".playing.tmp")
                tmp.writeText(json.toString())
                if (!tmp.renameTo(file)) { file.delete(); tmp.renameTo(file) }
            }
        }

        /** Removes the marker if it is still ours (another device may have taken over meanwhile). */
        fun clear() {
            val f = file
            val id = runCatching { JSONObject(f.readText()).optString("id") }.getOrNull()
            if (id == null || id == deviceId) f.delete()
        }
    }

    data class Holder(val device: String, val heartbeat: Long)

    const val HEARTBEAT_MS = 60_000L
    const val STALE_MS = 3 * 60_000L
}
