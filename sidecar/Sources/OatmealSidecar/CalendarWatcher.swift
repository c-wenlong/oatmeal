import EventKit
import Foundation
import SidecarProtocol

/// Reads the user's calendar through EventKit.
///
/// **EventKit rather than Google/Microsoft OAuth**, which is a deliberate
/// departure from the roadmap. macOS already holds the user's calendars —
/// Google, Exchange, iCloud, whatever they have added — and EventKit reads all
/// of them through one local API. Going direct to each provider would mean
/// registering OAuth clients, shipping a client secret inside a local-first app
/// that promises nothing leaves the machine, storing refresh tokens, and
/// handling two more auth flows. For an app whose entire premise is "your data
/// stays here", reading the calendar the OS already syncs is both less code and
/// a better promise.
///
/// The trade is that a user whose calendar is not in macOS Calendar sees
/// nothing. That is recoverable later by adding a provider path; shipping a
/// secret is not.
final class CalendarWatcher {
    /// How often the calendar is re-read.
    ///
    /// Five minutes, per the roadmap. Events are fetched for a window well
    /// ahead of now, so the poll interval only bounds how quickly a *newly
    /// created* meeting is noticed, not how punctually an existing one fires.
    private let interval: TimeInterval = 300

    /// How far ahead to look.
    private let horizon: TimeInterval = 24 * 3600

    private let store = EKEventStore()
    /// Handed both the window and the calendar list: the list is only ever
    /// wanted alongside the events, and passing it here keeps the caller from
    /// having to reach back into the watcher it is still constructing.
    private let onEvents: ([CalendarEvent], [CalendarSource]) -> Void
    private var timer: DispatchSourceTimer?
    private let queue = DispatchQueue(label: "oatmeal.sidecar.calendar")

    init(onEvents: @escaping ([CalendarEvent], [CalendarSource]) -> Void) {
        self.onEvents = onEvents
    }

    /// Whether the user has granted calendar access.
    static var isAuthorized: Bool {
        let status = EKEventStore.authorizationStatus(for: .event)
        if #available(macOS 14.0, *) {
            return status == .fullAccess
        }
        return status == .authorized
    }

    /// Asks for calendar access. Safe to call when already granted.
    func requestAccess(_ completion: @escaping (Bool) -> Void) {
        store.requestFullAccessToEvents { granted, error in
            if let error {
                Log.info("Calendar access failed: \(error)")
            }
            completion(granted)
        }
    }

    func start() {
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now(), repeating: interval)
        timer.setEventHandler { [weak self] in self?.poll() }
        timer.resume()
        self.timer = timer
        Log.info("Calendar watcher started.")
    }

    func stop() {
        timer?.cancel()
        timer = nil
    }

    private func poll() {
        guard Self.isAuthorized else {
            // Not an error worth shouting about on a loop: the user simply has
            // not granted access, and detection works without it.
            return
        }
        onEvents(fetch(), sources())
    }

    /// Reads the upcoming window.
    ///
    /// Every event is reported, meeting-shaped or not. The "is this a meeting"
    /// judgement lives in Rust, where it is pure and tested — this side stays a
    /// dumb reader so the rule can change without touching the sidecar.
    func fetch(now: Date = Date()) -> [CalendarEvent] {
        let predicate = store.predicateForEvents(
            withStart: now, end: now.addingTimeInterval(horizon), calendars: nil)

        return store.events(matching: predicate).compactMap { event in
            guard let start = event.startDate else { return nil }
            // All-day entries are holidays, birthdays and out-of-office blocks.
            // Offering to record one is never right.
            guard !event.isAllDay else { return nil }

            return CalendarEvent(
                id: event.eventIdentifier ?? UUID().uuidString,
                title: event.title,
                startsAt: Int(start.timeIntervalSince1970 * 1000),
                endsAt: event.endDate.map { Int($0.timeIntervalSince1970 * 1000) },
                location: event.location,
                // `url` is where Calendar.app puts a conferencing link; some
                // providers only put it in the notes, so both are sent and the
                // extraction happens on the Rust side.
                url: event.url?.absoluteString,
                notes: event.notes,
                attendeeCount: event.attendees?.count ?? 0,
                // Which calendar it came from. The predicate above still asks
                // for all of them — hiding one is a display choice, and making
                // it here would mean re-reading EventKit on every toggle.
                calendarId: event.calendar?.calendarIdentifier)
        }
    }

    /// Every calendar the account holds.
    ///
    /// Read alongside the events rather than on request: it changes about as
    /// often as they do, and the list is useless without them anyway.
    func sources() -> [CalendarSource] {
        guard Self.isAuthorized else { return [] }
        return store.calendars(for: .event).map { calendar in
            CalendarSource(
                id: calendar.calendarIdentifier,
                title: calendar.title,
                color: Self.hex(calendar.cgColor))
        }
    }

    /// `#rrggbb` for the dot beside a calendar's name.
    ///
    /// Converted to sRGB first: EventKit hands back whatever space the calendar
    /// was created in, and reading raw components from a non-RGB space gives
    /// colours that are simply wrong rather than merely approximate.
    static func hex(_ color: CGColor?) -> String? {
        guard let color,
            let srgb = CGColorSpace(name: CGColorSpace.sRGB),
            let converted = color.converted(to: srgb, intent: .defaultIntent, options: nil),
            let parts = converted.components, parts.count >= 3
        else { return nil }
        let byte = { (v: CGFloat) in Int((max(0, min(1, v)) * 255).rounded()) }
        return String(format: "#%02x%02x%02x", byte(parts[0]), byte(parts[1]), byte(parts[2]))
    }
}
