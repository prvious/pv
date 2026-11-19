// Issue #511 probe. Reproduces, without PV, the file operations and the FSEvents stream that the
// watcher tests use, and records every raw event the stream delivers.
//
// Each round:
//   1. setup: create `project/`, then save `project/pv.yml` the way PV's write_sensitive_file
//      does (write temp file, chmod 0600, rename over pv.yml, chmod 0600).
//   2. wait a chosen gap, then start a stream on `project/` exactly as notify 8.2.0 does
//      (since-now, latency 0, FileEvents | NoDefer), plus MarkSelf so our own events are flagged.
//   3. wait 150 ms: any event about pv.yml now is a *late setup event* (failure 2's suspect).
//   4. save pv.yml again the same way (the test's edit) and record how its events spread over
//      callbacks and time (failure 3's first suspect: one save split by more than 50 ms).
//
// Usage: fsevents-probe <rounds> <scratch-dir> <label>
// Prints one JSON object per round on stdout.

import CoreServices
import Foundation

func nowMs() -> Double { Double(clock_gettime_nsec_np(CLOCK_UPTIME_RAW)) / 1_000_000 }

struct RawEvent {
    let arrivedMs: Double
    let id: UInt64
    let flags: UInt32
    let path: String
    let callback: Int
}

final class Recorder {
    let lock = NSLock()
    var events: [RawEvent] = []
    var callbacks = 0
}

let callback: FSEventStreamCallback = { _, info, count, paths, flags, ids in
    let arrived = nowMs()
    let recorder = Unmanaged<Recorder>.fromOpaque(info!).takeUnretainedValue()
    let cPaths = paths.assumingMemoryBound(to: UnsafePointer<CChar>.self)
    recorder.lock.lock()
    recorder.callbacks += 1
    for index in 0..<count {
        recorder.events.append(RawEvent(
            arrivedMs: arrived, id: ids[index], flags: flags[index],
            path: String(cString: cPaths[index]), callback: recorder.callbacks))
    }
    recorder.lock.unlock()
}

final class Stream {
    var runLoop: CFRunLoop?
    let done = DispatchSemaphore(value: 0)

    init(path: String, recorder: Recorder) {
        let ready = DispatchSemaphore(value: 0)
        var context = FSEventStreamContext(
            version: 0, info: Unmanaged.passUnretained(recorder).toOpaque(),
            retain: nil, release: nil, copyDescription: nil)
        let flags = FSEventStreamCreateFlags(
            kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagNoDefer
                | kFSEventStreamCreateFlagMarkSelf)
        let stream = FSEventStreamCreate(
            kCFAllocatorDefault, callback, &context, [path] as CFArray,
            FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 0.0, flags)!
        let thread = Thread { [self] in
            let current = CFRunLoopGetCurrent()!
            FSEventStreamScheduleWithRunLoop(stream, current, CFRunLoopMode.defaultMode.rawValue)
            FSEventStreamStart(stream)
            self.runLoop = current
            ready.signal()
            CFRunLoopRun()
            FSEventStreamStop(stream)
            FSEventStreamInvalidate(stream)
            FSEventStreamRelease(stream)
            self.done.signal()
        }
        thread.start()
        ready.wait()
    }

    func stop() {
        guard let runLoop else { return }
        while !CFRunLoopIsWaiting(runLoop) { sched_yield() }
        CFRunLoopStop(runLoop)
        done.wait()
    }
}

var temporaryCounter = 0

func check(_ result: Int32, _ what: String) {
    if result != 0 { fatalError("\(what): errno \(errno)") }
}

/// PV's `write_sensitive_file`: temp file, chmod 0600, owner/mode checks, rename, chmod, checks.
func saveLikePV(_ path: String, _ content: String) {
    temporaryCounter += 1
    let temporary = "\(path).\(getpid()).\(temporaryCounter).tmp"
    let fd = open(temporary, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0o666)
    if fd < 0 { fatalError("open: errno \(errno)") }
    let bytes = Array(content.utf8)
    _ = bytes.withUnsafeBytes { write(fd, $0.baseAddress, $0.count) }
    close(fd)
    var info = stat()
    check(chmod(temporary, 0o600), "chmod tmp")
    check(stat(temporary, &info), "stat tmp")
    check(rename(temporary, path), "rename")
    check(stat(path, &info), "stat owner")
    check(chmod(path, 0o600), "chmod")
    check(stat(path, &info), "stat mode")
}

func hex(_ value: UInt32) -> String { "0x" + String(value, radix: 16) }

let arguments = CommandLine.arguments
let rounds = Int(arguments[1])!
// realpath, not Foundation: Foundation drops `/private`, but FSEvents reports `/private/var/...`.
let scratch = String(cString: realpath(arguments[2], nil)!)
let label = arguments[3]
let gaps: [Double] = [0, 0, 1, 3, 10, 30]
let ownFlag = UInt32(kFSEventStreamEventFlagOwnEvent)

for round in 0..<rounds {
    let gapMs = gaps[round % gaps.count]
    let project = "\(scratch)/\(label)-\(round)/project"
    let config = "\(project)/pv.yml"
    check(mkdir("\(scratch)/\(label)-\(round)", 0o777), "mkdir round")
    check(mkdir(project, 0o777), "mkdir project")
    check(chmod(project, 0o700), "chmod project")
    let setupStart = nowMs()
    saveLikePV(config, "php: '8.3'\n")
    let setupEnd = nowMs()
    if ProcessInfo.processInfo.environment["PROBE_OTHER_WRITE"] != nil {
        // What the real test does between its save and watching: open its SQLite database, which
        // writes outside the watched directory.
        let other = open("\(scratch)/\(label)-\(round)/other.db", O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0o600)
        _ = write(other, "x", 1)
        fsync(other)
        close(other)
    }
    if gapMs > 0 { usleep(useconds_t(gapMs * 1000)) }

    // The fix under test: wait until a marker written now reaches a separate stream on the
    // round's directory, then start the real stream.
    let roundDirectory = "\(scratch)/\(label)-\(round)"
    let marker = "\(roundDirectory)/marker"
    let markerRecorder = Recorder()
    let markerStream = Stream(path: roundDirectory, recorder: markerRecorder)
    let markerWriteStart = nowMs()
    let markerFd = open(marker, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0o600)
    _ = write(markerFd, "x", 1)
    close(markerFd)
    var markerId: UInt64? = nil
    var markerEvents: [RawEvent] = []
    for _ in 0..<5000 {
        markerRecorder.lock.lock()
        markerEvents = markerRecorder.events
        markerRecorder.lock.unlock()
        if let event = markerEvents.first(where: { $0.path == marker }) {
            markerId = event.id
            break
        }
        usleep(1000)
    }
    let markerWaitMs = nowMs() - markerWriteStart
    guard let markerId else { fatalError("marker never arrived") }
    // The marker's stream keeps running until the round ends, so a setup write numbered after the
    // marker is still seen even if it arrives after the real stream starts.

    let recorder = Recorder()
    let idBeforeStart = FSEventsGetCurrentEventId()
    let startBegin = nowMs()
    let stream = Stream(path: project, recorder: recorder)
    let started = nowMs()
    let idAfterStart = FSEventsGetCurrentEventId()

    usleep(150_000)
    // The edit's own events are numbered after this, which keeps them out of the setup counts.
    let idAtActionStart = FSEventsGetCurrentEventId()
    let actionStart = nowMs()
    saveLikePV(config, "php: '8.4'\n")
    let actionEnd = nowMs()
    usleep(250_000)
    stream.stop()
    markerStream.stop()
    markerRecorder.lock.lock()
    markerEvents = markerRecorder.events
    markerRecorder.lock.unlock()
    // Setup events the marker stream received late: their numbers must be below the marker's.
    let setupSeen = markerEvents.filter { $0.path.hasPrefix(project) && $0.id <= idAtActionStart }
    let setupAfterMarker = setupSeen.filter { $0.id > markerId }.count
    let setupSeenByMarkerStream = setupSeen.count

    recorder.lock.lock()
    let events = recorder.events
    let callbacks = recorder.callbacks
    recorder.lock.unlock()

    // Only arriving before the edit began proves an event came from setup. The edit's events start
    // with its temporary file; without that marker, or after FSEvents reports dropped events,
    // nothing else in the round can be attributed, so the round is ambiguous and the summary
    // leaves it out of the edit's numbers.
    let actionTemporary = "\(config).\(getpid()).\(temporaryCounter).tmp"
    let markerIndex = events.firstIndex { $0.path == actionTemporary }
    let droppedFlags = UInt32(kFSEventStreamEventFlagMustScanSubDirs | kFSEventStreamEventFlagUserDropped | kFSEventStreamEventFlagKernelDropped)
    let dropped = events.contains { $0.flags & droppedFlags != 0 }
    let late = events.filter { $0.arrivedMs < actionStart }
    let lateAfterEditStart = markerIndex.map { events[..<$0].filter { $0.arrivedMs >= actionStart } } ?? []
    let action = markerIndex.map { Array(events[$0...]) } ?? []
    let actionConfig = action.filter { $0.path == config || $0.path == project }
    let configCallbacks = Set(action.filter { $0.path == config }.map(\.callback))
    let configArrivals = action.filter { $0.path == config }.map(\.arrivedMs)

    func describe(_ event: RawEvent) -> [String: Any] {
        [
            "id": event.id, "flags": hex(event.flags), "own": event.flags & ownFlag != 0,
            "path": String(event.path.dropFirst(project.count)), "callback": event.callback,
            "after_start_ms": event.arrivedMs - started,
            "after_action_end_ms": event.arrivedMs - actionEnd,
        ]
    }

    let record: [String: Any] = [
        "label": label, "round": round, "gap_ms": gapMs,
        "setup_ms": setupEnd - setupStart,
        "setup_end_to_stream_start_ms": started - setupEnd,
        "stream_start_ms": started - startBegin,
        "id_before_start": idBeforeStart, "id_after_start": idAfterStart,
        "callbacks": callbacks,
        "late_events": late.map(describe),
        "late_config_events": late.filter { $0.path == config }.count,
        "action_events": action.count,
        "action_config_callbacks": configCallbacks.count,
        "action_config_spread_ms": (configArrivals.max() ?? 0) - (configArrivals.min() ?? 0),
        "action_first_lag_ms": (actionConfig.map(\.arrivedMs).min() ?? actionEnd) - actionEnd,
        "action_last_lag_ms": (actionConfig.map(\.arrivedMs).max() ?? actionEnd) - actionEnd,
        "ambiguous": markerIndex == nil || dropped, "dropped": dropped,
        "setup_events_after_edit_start": lateAfterEditStart.map(describe),
        // An ambiguous round keeps every raw event, since none of its later ones can be attributed.
        "raw_events": markerIndex == nil || dropped ? events.map(describe) : [],
        "marker_wait_ms": markerWaitMs,
        "setup_seen_by_marker_stream": setupSeenByMarkerStream,
        "setup_numbered_after_marker": setupAfterMarker,
        "marker_id": markerId,
        "id_at_action_start": idAtActionStart,
        "marker_stream_events": markerEvents.map {
            ["id": $0.id, "flags": hex($0.flags), "path": String($0.path.dropFirst(roundDirectory.count))]
        },
        "real_events_at_or_below_marker": events.filter { $0.id <= markerId }.count,
        "action_ms": actionEnd - actionStart,
        "action_events_detail": configCallbacks.count > 1 ? action.map(describe) : [],
    ]
    let data = try! JSONSerialization.data(withJSONObject: record, options: [.sortedKeys])
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write("\n".data(using: .utf8)!)
}
