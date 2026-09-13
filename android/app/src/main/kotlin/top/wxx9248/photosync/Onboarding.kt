package top.wxx9248.photosync

import android.content.Context

/**
 * What the application has already said to the person holding the phone.
 *
 * `SPEC.md` §3.5 keeps the pairing credential and nothing about a transfer, and this is
 * neither: no progress, no catalog, nothing a desktop could disagree with. It is one fact
 * about a conversation, and without it §3.1's fourth step --- a page of settings only a person
 * can change --- would appear again on every launch.
 */
internal class Onboarding(context: Context) {
    private val remembered =
        context.getSharedPreferences("onboarding", Context.MODE_PRIVATE)

    /**
     * Whether this step has been put to a person yet.
     *
     * Asked, not granted. §3.1 lets somebody decline media management and the battery
     * exemption, and a step that reappeared on every launch would be asking them to decline
     * it again every time --- which is nagging rather than onboarding.
     */
    fun asked(step: Step): Boolean = remembered.getBoolean(step.key, false)

    fun remember(step: Step) {
        remembered.edit().putBoolean(step.key, true).apply()
    }

    /** The steps of §3.1 that are answered once rather than checked each time. */
    enum class Step(val key: String) {
        MEDIA_MANAGEMENT("media-management-asked"),
        BATTERY_EXEMPTION("battery-exemption-asked"),
        BRAND_STEPS("brand-steps-seen"),
    }
}
