package nl.bvdm.rukoo.ui.root

import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.lifecycle.SavedStateHandle
import java.io.File
import java.nio.file.Files
import java.time.ZoneId
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import nl.bvdm.rukoo.AppServices
import nl.bvdm.rukoo.NotificationRoute
import nl.bvdm.rukoo.R
import nl.bvdm.rukoo.agents.AgentIntents
import nl.bvdm.rukoo.agents.AgentSettingsStore
import nl.bvdm.rukoo.core.Change
import nl.bvdm.rukoo.mail.MailEngine
import nl.bvdm.rukoo.mail.MailException
import nl.bvdm.rukoo.mail.compose.toOutgoing
import nl.bvdm.rukoo.mail.model.Account
import nl.bvdm.rukoo.mail.model.AccountPatch
import nl.bvdm.rukoo.mail.model.FullMessage
import nl.bvdm.rukoo.mail.model.OutgoingMessage
import nl.bvdm.rukoo.mail.sync.MailNotifier
import nl.bvdm.rukoo.ui.Mailto
import nl.bvdm.rukoo.agents.preview.PreviewAgentHub
import nl.bvdm.rukoo.core.SecretCipher
import nl.bvdm.rukoo.desktop.StubDesktopLink
import nl.bvdm.rukoo.mail.demo.PreviewMailEngine
import nl.bvdm.rukoo.mail.model.Address
import nl.bvdm.rukoo.mail.model.MailView
import nl.bvdm.rukoo.mail.model.ReplyKind
import nl.bvdm.rukoo.mail.model.ViewKind
import nl.bvdm.rukoo.mcp.StubMcpController
import nl.bvdm.rukoo.settings.AppSettings
import nl.bvdm.rukoo.settings.SettingsStore
import nl.bvdm.rukoo.ui.ComposerController
import nl.bvdm.rukoo.ui.ComposerState
import nl.bvdm.rukoo.ui.ScreenState
import nl.bvdm.rukoo.ui.composer.DraftStore
import nl.bvdm.rukoo.ui.mail.DialogChoice
import nl.bvdm.rukoo.ui.mail.UiStrings
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class AppViewModelTest {
    private val main = UnconfinedTestDispatcher()

    @Before
    fun setUp() = Dispatchers.setMain(main)

    @After
    fun tearDown() = Dispatchers.resetMain()

    // Strings as "id|args", so a test can tell which text was chosen.
    private object Fake : UiStrings {
        override fun get(id: Int, vararg args: Any) = (listOf(id) + args).joinToString("|")
        override fun plural(id: Int, count: Int, vararg args: Any) = (listOf(id) + args).joinToString("|")
    }

    private class Services(
        dir: File,
        scope: CoroutineScope,
        override val drafts: DraftStore,
        val preview: PreviewMailEngine = PreviewMailEngine(zone = ZoneId.of("Europe/Amsterdam")),
        override val mail: MailEngine = preview,
    ) : AppServices {
        override val agents = PreviewAgentHub(scope) { 1761130000000 }
        override val composer = ComposerController()
        override val screen = ScreenState()
        override val settings = SettingsStore(PreferenceDataStoreFactory.create(scope = scope) { File(dir, "settings.preferences_pb") })
        override val settingsSnapshot = MutableStateFlow(AppSettings())
        override val agentSettings = AgentSettingsStore(
            File(dir, "agents.json"),
            object : SecretCipher {
                override fun encrypt(plain: String) = plain
                override fun decrypt(blob: String) = blob
            },
        )
        override val desktop = StubDesktopLink(agentSettings)
        override val mcp = StubMcpController()
    }

    private fun TestScope.setup(
        twoPane: Boolean = false,
        drafts: DraftStore = DraftStore(null),
        preview: PreviewMailEngine = PreviewMailEngine(zone = ZoneId.of("Europe/Amsterdam")),
        mail: MailEngine = preview,
    ): Pair<Services, AppViewModel> {
        val services = Services(Files.createTempDirectory("rukoo-vm").toFile(), backgroundScope, drafts, preview, mail)
        val vm = AppViewModel(services)
        vm.mailUi.strings = Fake
        vm.onLayout(twoPane)
        return services to vm
    }

    @Test
    fun openingAnEmailMarksItReadAndTellsTheTools() = runTest(main) {
        val (s, vm) = setup()
        val unread = s.mail.listMessages(MailView.AllInboxes).first { it.unread }
        vm.openMessage(unread.id)
        assertFalse(s.mail.summary(unread.id)!!.unread)
        assertEquals(Pane.READER, vm.state.value.top)
        assertEquals(unread.id, s.screen.current.openMessageId)
        vm.toggleChecked("x")
        assertEquals(listOf("x"), s.screen.current.selectedIds)
    }

    @Test
    fun anEmailOpenedWhileUnreadIsKeptBeforeItIsMarkedRead() = runTest(main) {
        val preview = PreviewMailEngine(zone = ZoneId.of("Europe/Amsterdam"))
        lateinit var vm: AppViewModel
        // What the Unread filter would have kept when the read flag reached the list.
        var keptWhenRead: String? = "not marked"
        val mail = object : MailEngine by preview {
            override suspend fun setFlags(ids: List<String>, unread: Boolean?, starred: Boolean?, answered: Boolean?) {
                if (unread == false) keptWhenRead = vm.state.value.keptId
                preview.setFlags(ids, unread, starred, answered)
            }
        }
        val (s, model) = setup(preview = preview, mail = mail)
        vm = model
        vm.setFilter(nl.bvdm.rukoo.mail.model.QuickFilter.UNREAD)
        val list = s.mail.listMessages(MailView.AllInboxes)
        val first = list.first { it.unread }
        vm.openMessage(first.id)
        assertEquals(first.id, keptWhenRead)
        assertFalse(s.mail.summary(first.id)!!.unread)
        // Another unread email takes its place; a read one is kept by nobody.
        val second = list.last { it.unread && it.id != first.id }
        vm.openMessage(second.id)
        assertEquals(second.id, vm.state.value.keptId)
        keptWhenRead = "not marked"
        vm.openMessage(list.first { !it.unread }.id)
        assertNull(vm.state.value.keptId)
        assertEquals("a read email isn't marked again", "not marked", keptWhenRead)
    }

    @Test
    fun archivingTheOpenEmailGoesOnToTheNextAndUndoBringsItBack() = runTest(main) {
        val (s, vm) = setup(twoPane = true)
        val list = s.mail.listMessages(MailView.AllInboxes)
        vm.listShown(list.map { it.id })
        vm.openMessage(list[1].id)
        vm.mailUi.archive(listOf(list[1]))
        assertEquals(list[2].id, vm.state.value.openMessageId)
        vm.mailUi.toasts.current!!.onAction!!.invoke()
        // The restored copy has a new id and opens again, as it was open.
        val back = vm.state.value.openMessageId!!
        assertEquals(list[1].messageId, s.mail.summary(back)!!.messageId)
    }

    @Test
    fun backOnAComposerWithWordsAsksAndDontSaveDiscards() = runTest(main) {
        val (s, vm) = setup()
        vm.openComposer(ComposerState(subject = "Plan", body = "Thursday?", dirty = true))
        assertTrue(vm.backAsks())
        assertTrue(vm.back())
        val ask = vm.mailUi.dialog!!
        assertEquals("${R.string.composer_close_title}", ask.title)
        assertTrue(vm.state.value.composerOpen)
        ask.answer(DialogChoice.OTHER)
        assertFalse(vm.state.value.composerOpen)
        assertNull(s.composer.current())
        assertTrue(s.drafts.drafts.value.isEmpty())
    }

    @Test
    fun saveKeepsTheDraftAndACleanReopenStaysInDrafts() = runTest(main) {
        val (s, vm) = setup()
        val key = s.composer.open(ComposerState(to = listOf(Address("", "sanne@example.com")), body = "Notes", dirty = true))
        vm.closeComposer()
        vm.mailUi.dialog!!.answer(DialogChoice.CONFIRM)
        assertNull(s.composer.current())
        assertEquals(key, s.drafts.drafts.value.single().id)
        vm.openLocalDraft(key)
        assertFalse(s.composer.current()!!.dirty)
        vm.closeComposer()
        assertNull(vm.mailUi.dialog)
        assertEquals("${R.string.composer_status_kept}", vm.mailUi.toasts.current!!.message)
        assertEquals(1, s.drafts.drafts.value.size)
    }

    @Test
    fun anAgentsUntouchedDraftStillAsks() = runTest(main) {
        val (s, vm) = setup()
        s.composer.open(ComposerState(subject = "Re: Call", body = "Thursday works.", draftedBy = "Clark"))
        assertTrue(vm.composerAsks())
    }

    @Test
    fun anotherComposerParksTheUnsavedOne() = runTest(main) {
        val (s, vm) = setup()
        val first = s.composer.open(ComposerState(body = "half written", dirty = true))
        vm.openComposer(ComposerState(subject = "New"))
        assertEquals(first, s.drafts.drafts.value.single().id)
        assertEquals("New", s.composer.current()!!.subject)
    }

    @Test
    fun sendNeedsARecipientAsksAboutNoSubjectThenSends() = runTest(main) {
        val (s, vm) = setup()
        vm.newMessage()
        vm.send()
        assertEquals("${R.string.composer_errors_no_recipients}", vm.mailUi.toasts.current!!.message)
        s.composer.update { it.copy(to = listOf(Address("Sanne", "sanne@example.com")), body = "Hi", dirty = true) }
        vm.send()
        vm.mailUi.dialog!!.answer(DialogChoice.CONFIRM)
        assertNull(s.composer.current())
        assertEquals("${R.string.composer_status_sent}", vm.mailUi.toasts.current!!.message)
        assertEquals("Hi", s.mail.listMessages(MailView(ViewKind.SENT)).first().preview)
    }

    @Test
    fun addressesStillBeingTypedGoAlongWithSend() = runTest(main) {
        val (s, vm) = setup()
        s.composer.open(
            ComposerState(accountId = "demo", to = listOf(Address("Sanne", "sanne@example.com")), subject = "Plan", body = "Hi", typed = mapOf("cc" to "joris@example.com")),
        )
        vm.send()
        val sent = s.mail.listMessages(MailView(ViewKind.SENT)).first()
        assertEquals(listOf("joris@example.com"), sent.cc.map { it.address })
        assertNull(s.composer.current())
    }

    @Test
    fun typedRecipientTextCountsAsUnsavedWords() = runTest(main) {
        val (s, vm) = setup()
        s.composer.open(ComposerState())
        assertFalse(vm.composerAsks())
        s.composer.update { it.copy(typed = mapOf("to" to "sanne@exa")) }
        assertTrue(vm.composerAsks())
    }

    @Test
    fun aDraftHoldingOnlyTypedTextIsParkedWithItAsRecipient() = runTest(main) {
        val (s, vm) = setup()
        val first = s.composer.open(ComposerState(typed = mapOf("to" to "sanne@example.com")))
        vm.openComposer(ComposerState(subject = "New"))
        val parked = s.drafts.drafts.value.single()
        assertEquals(first, parked.id)
        assertEquals(listOf("sanne@example.com"), parked.to.map { it.address })
        assertTrue(parked.typed.isEmpty())
        assertEquals("New", s.composer.current()!!.subject)
    }

    @Test
    fun anAgentsDraftKeptInDraftsDoesNotAskAgain() = runTest(main) {
        val (s, vm) = setup()
        val key = s.composer.open(ComposerState(subject = "Re: Call", body = "Thursday works.", draftedBy = "Clark"))
        vm.saveDraft()
        vm.openLocalDraft(key)
        assertFalse(vm.composerAsks())
        // Another draft arriving doesn't park it a second time either.
        vm.openComposer(ComposerState(subject = "New"))
        assertEquals(1, s.drafts.drafts.value.size)
        assertFalse(vm.mailUi.toasts.current?.message == "${R.string.composer_status_saved_closed}")
    }

    @Test
    fun anEditDuringTheSaveIsSavedTooBeforeTheComposerCloses() = runTest(main) {
        // Writes wait for the test's scheduler, so the user can type while the file is being written.
        val io = StandardTestDispatcher(testScheduler)
        val drafts = DraftStore(File(Files.createTempDirectory("rukoo-drafts").toFile(), DraftStore.FILE_NAME), io)
        val (s, vm) = setup(drafts = drafts)
        val key = s.composer.open(ComposerState(body = "first", dirty = true))
        vm.saveDraft()
        assertNotNull(s.composer.current())
        s.composer.update { it.copy(body = "first and more", dirty = true) }
        advanceUntilIdle()
        assertNull(s.composer.current())
        assertEquals("first and more", drafts.get(key)!!.body)
        assertEquals("${R.string.composer_status_saved_to_drafts}", vm.mailUi.toasts.current!!.message)
    }

    @Test
    fun aDraftReplacedDuringTheSaveStaysOpen() = runTest(main) {
        val io = StandardTestDispatcher(testScheduler)
        val drafts = DraftStore(File(Files.createTempDirectory("rukoo-drafts").toFile(), DraftStore.FILE_NAME), io)
        val (s, vm) = setup(drafts = drafts)
        s.composer.open(ComposerState(body = "first", dirty = true))
        vm.saveDraft()
        // An agent's write_draft opens its own draft while the first one is being written.
        s.composer.open(ComposerState(body = "From the agent", draftedBy = "Clark"))
        advanceUntilIdle()
        assertEquals("From the agent", s.composer.current()!!.body)
        assertEquals(1, drafts.drafts.value.size)
    }

    @Test
    fun aDraftOpenedMeanwhileIsNotReplacedByAnOlderCopy() = runTest(main) {
        val io = StandardTestDispatcher(testScheduler)
        val drafts = DraftStore(File(Files.createTempDirectory("rukoo-drafts").toFile(), DraftStore.FILE_NAME), io)
        val (s, vm) = setup(drafts = drafts)
        s.composer.open(ComposerState(body = "A", dirty = true))
        // Opening draft B parks A first; while A is written, an agent opens a newer B.
        vm.openComposer(ComposerState(id = "b", body = "B as kept"))
        s.composer.open(ComposerState(id = "b", body = "B rewritten", draftedBy = "Clark"))
        advanceUntilIdle()
        assertEquals("B rewritten", s.composer.current()!!.body)
        assertEquals("A", drafts.drafts.value.single().body)
    }

    @Test
    fun scrollPositionsOfEveryListSurviveRecreation() = runTest(main) {
        val (s, _) = setup()
        val saved = SavedStateHandle()
        val before = AppViewModel(s, saved)
        before.keepListPosition("inbox", 12, 30)
        before.keepListPosition("archive", 3, 0)
        val after = AppViewModel(s, saved)
        assertEquals(12 to 30, after.listPosition("inbox"))
        assertEquals(3 to 0, after.listPosition("archive"))
    }

    @Test
    fun tappingTheOpenDraftAgainKeepsItsEdits() = runTest(main) {
        val (s, vm) = setup()
        val key = s.composer.open(ComposerState(body = "first", dirty = true))
        vm.saveDraft()
        vm.openLocalDraft(key)
        s.composer.update { it.copy(body = "first and more", dirty = true) }
        vm.openLocalDraft(key)
        assertEquals("first and more", s.composer.current()!!.body)
    }

    @Test
    fun aReplyStartsWithTheReplyFieldsAndAQuote() = runTest(main) {
        val (s, vm) = setup()
        val sanne = s.mail.listMessages(MailView.AllInboxes).first { it.subject == "Call on Thursday" }
        vm.reply(sanne.id, ReplyKind.REPLY_ALL)
        val d = assertNotNullAndGet(s.composer.current())
        assertEquals("Re: Call on Thursday", d.subject)
        assertEquals(listOf("sanne@example.com"), d.to.map { it.address })
        assertEquals(listOf("joris@example.com"), d.cc.map { it.address })
        // The engine's quote: desktop's marked header and the original in a blockquote.
        assertTrue(d.quote.startsWith("<br><div data-rukoo-quote"))
        assertTrue(d.quote.contains("-------- Original message --------"))
        assertTrue(d.quote.contains("<blockquote"))
        assertEquals(sanne.messageId, d.inReplyTo)
        assertTrue(vm.state.value.composerOpen)
    }

    @Test
    fun theSignatureStaysOutOfTheTextSoAnAgentsDraftKeepsIt() = runTest(main) {
        val (s, vm) = setup()
        s.mail.updateAccount("demo", AccountPatch(signature = Change("Demo User\nRukoo Mail")))
        vm.newMessage()
        // An untouched new message has no text of its own: write_draft treats it as free, not as the user's words.
        assertEquals("", s.composer.current()!!.body)
        assertFalse(vm.composerAsks())
        // What write_draft does: the agent's text replaces the body.
        s.composer.update { it.copy(to = listOf(Address("Sanne", "sanne@example.com")), subject = "Plan", body = "Thursday works.", draftedBy = "Clark") }
        vm.send()
        val sent = s.mail.listMessages(MailView(ViewKind.SENT)).first { it.subject == "Plan" }
        val html = s.mail.fullMessage(sent.id).html
        assertTrue(html.contains("<div>Thursday works.</div><div><br></div><div class=\"signature\">Demo User<br>Rukoo Mail</div>"))
    }

    @Test
    fun aSignatureLeftOutStaysOut() = runTest(main) {
        val (s, vm) = setup()
        s.mail.updateAccount("demo", AccountPatch(signature = Change("Demo User")))
        vm.newMessage()
        s.composer.update { it.copy(to = listOf(Address("Sanne", "sanne@example.com")), subject = "Short", body = "Ok", signatureExcluded = true) }
        vm.send()
        val sent = s.mail.listMessages(MailView(ViewKind.SENT)).first { it.subject == "Short" }
        assertFalse(s.mail.fullMessage(sent.id).html.contains("class=\"signature\""))
    }

    @Test
    fun anEditedServerDraftKeepsItsQuoteAndReplacesTheStoredCopy() = runTest(main) {
        val (s, vm) = setup()
        val sanne = s.mail.listMessages(MailView.AllInboxes).first { it.subject == "Call on Thursday" }
        // A reply saved in Drafts (as desktop would), opened again with Edit.
        val reply = s.mail.replyFields(sanne.id, ReplyKind.REPLY).toComposerState().copy(body = "Thursday at 10?")
        val stored = s.mail.saveDraft(reply.toOutgoing("demo", Address("Demo User", "demo@example.com")))!!
        vm.openMessage(stored)
        vm.editDraft(stored)
        val d = s.composer.current()!!
        assertEquals(stored, d.draftId)
        assertEquals("Thursday at 10?", d.body)
        assertTrue(d.quote.contains("<blockquote"))
        assertEquals("demo@example.com", d.from!!.address)

        // Saving it again replaces the stored copy instead of adding one on this phone.
        s.composer.update { it.copy(body = "Thursday at 11?", dirty = true) }
        vm.saveDraft()
        val drafts = s.mail.listMessages(MailView(ViewKind.DRAFTS))
        assertNull(s.mail.summary(stored))
        val again = drafts.single { it.subject == "Re: Call on Thursday" }
        assertTrue(s.drafts.drafts.value.isEmpty())
        // The reader that showed the old copy shows the new one.
        assertEquals(again.id, vm.state.value.openMessageId)

        // Sending it removes the draft from Drafts, and from the reader.
        vm.editDraft(again.id)
        s.composer.update { it.copy(body = "Thursday at 11 then.", dirty = true) }
        vm.send()
        assertNull(s.mail.summary(again.id))
        assertNull(vm.state.value.openMessageId)
        val sent = s.mail.listMessages(MailView(ViewKind.SENT)).first { it.subject == "Re: Call on Thursday" }
        assertTrue(s.mail.fullMessage(sent.id).html.contains("data-rukoo-quote"))
    }

    @Test
    fun notificationTapsOpenTheMessageTheInboxOrTheChat() = runTest(main) {
        val (s, vm) = setup()
        val m = s.mail.listMessages(MailView.AllInboxes).first()
        // The new-mail notifier's extras, read the way MainActivity reads them.
        val one = MailNotifier.extras("demo", listOf(m))
        vm.openRoute(NotificationRoute.of({ one[it] }, { false })!!)
        assertEquals(m.id, vm.state.value.openMessageId)
        assertEquals(Pane.READER, vm.state.value.top)

        val several = MailNotifier.extras("demo", listOf(m, m.copy(id = "other")))
        assertEquals(NotificationRoute.Inbox("demo"), NotificationRoute.of({ several[it] }, { false }))
        vm.openRoute(NotificationRoute.Inbox("demo"))
        assertEquals(MailView(ViewKind.INBOX), vm.state.value.view)

        // The agent notifier's extras: a draft notification whose composer has closed opens its chat.
        val agent = mapOf(AgentIntents.EXTRA_CONVERSATION_ID to "c_1", AgentIntents.EXTRA_MESSAGE_ID to m.id)
        val route = NotificationRoute.of({ agent[it] }, { it == AgentIntents.EXTRA_OPEN_COMPOSER })!!
        assertEquals(NotificationRoute.Composer(NotificationRoute.Chat("c_1", m.id)), route)
        vm.openRoute(route)
        assertTrue(vm.state.value.chatOpen)
        assertEquals("c_1", vm.state.value.chatConversationId)
    }

    @Test
    fun aMailtoLinkGetsTheDefaultAccountAndFrom() = runTest(main) {
        val (s, vm) = setup()
        vm.newMessage(Mailto.withExtras(Mailto.parse("mailto:"), to = arrayOf("help@x.com"), subject = "Bug"))
        val d = s.composer.current()!!
        assertEquals("demo", d.accountId)
        assertEquals("demo@example.com", d.from!!.address)
        assertEquals(listOf("help@x.com"), d.to.map { it.address })
        assertEquals("Bug", d.subject)
    }

    @Test
    fun followingAnEmailDoesntBringTheChatForward() = runTest(main) {
        val (_, vm) = setup(twoPane = true)
        vm.openChat("c_1", "a")
        vm.interacted(Pane.READER)
        vm.followEmail("c_2", "b")
        assertEquals("c_2", vm.state.value.chatConversationId)
        assertEquals("b", vm.state.value.chatEmailId)
        assertEquals(Pane.READER, vm.state.value.lastInteracted)
        vm.openMessage("x")
        vm.openChat(withoutEmail = true)
        assertNull(vm.state.value.chatEmailId)
        assertNull(vm.state.value.chatConversationId)
    }

    @Test
    fun aServerDraftSavedTwiceInARowKeepsItsAttachments() = runTest(main) {
        val preview = PreviewMailEngine(zone = ZoneId.of("Europe/Amsterdam"))
        var duringSave: (() -> Unit)? = null
        // The user types while the save is under way, so keepAndClose saves a second time.
        val mail = object : MailEngine by preview {
            override suspend fun saveDraft(message: OutgoingMessage): String? = preview.saveDraft(message).also {
                duringSave?.invoke()
                duringSave = null
            }
        }
        val (s, vm) = setup(preview = preview, mail = mail)
        val sanne = s.mail.listMessages(MailView.AllInboxes).first { it.subject == "Call on Thursday" }
        val forward = s.mail.replyFields(sanne.id, ReplyKind.FORWARD).toComposerState()
            .copy(to = listOf(Address("", "joris@example.com")), bcc = listOf(Address("", "me@example.com")), body = "FYI")
        val stored = s.mail.saveDraft(forward.toOutgoing("demo", Address("Demo User", "demo@example.com")))!!
        vm.editDraft(stored)
        // Reopened, its attachment comes from the stored draft, and its Bcc is still there.
        assertEquals(stored, s.composer.current()!!.attachments.single().sourceMessageId)
        assertEquals(listOf("me@example.com"), s.composer.current()!!.bcc.map { it.address })
        duringSave = { s.composer.update { it.copy(body = it.body + " Thanks", dirty = true) } }
        s.composer.update { it.copy(body = "FYI.", dirty = true) }
        vm.saveDraft()
        assertNull(s.composer.current())
        // Both saves reached the server; nothing fell back to this phone.
        assertTrue(s.drafts.drafts.value.isEmpty())
        val saved = s.mail.listMessages(MailView(ViewKind.DRAFTS)).single { it.subject == "Fwd: Call on Thursday" }
        val full = s.mail.fullMessage(saved.id)
        assertTrue(full.text.startsWith("FYI. Thanks"))
        assertEquals(listOf("Proposal-v3.pdf"), full.attachments.map { it.filename })
    }

    /** A forward kept in Drafts, opened with Edit, saved once while the new copy can't be read back. */
    private suspend fun TestScope.unsettledDraft(): Triple<Services, AppViewModel, () -> Unit> {
        val preview = PreviewMailEngine(zone = ZoneId.of("Europe/Amsterdam"))
        var newCopy: String? = null
        var offline = true
        val mail = object : MailEngine by preview {
            override suspend fun saveDraft(message: OutgoingMessage): String? = preview.saveDraft(message).also { newCopy = it }

            override suspend fun fullMessage(id: String) = if (offline && id == newCopy) throw MailException("offline") else preview.fullMessage(id)
        }
        val (s, vm) = setup(preview = preview, mail = mail)
        val sanne = s.mail.listMessages(MailView.AllInboxes).first { it.subject == "Call on Thursday" }
        val forward = s.mail.replyFields(sanne.id, ReplyKind.FORWARD).toComposerState().copy(to = listOf(Address("", "joris@example.com")))
        val stored = s.mail.saveDraft(forward.toOutgoing("demo", Address("Demo User", "demo@example.com")))!!
        vm.editDraft(stored)
        s.composer.update { it.copy(body = "FYI", dirty = true) }
        vm.saveDraft()
        // Saved, but not closed as if all was well, and nothing kept on this phone with references to a deleted copy.
        val d = s.composer.current()!!
        assertEquals(newCopy, d.draftId)
        assertEquals(stored, d.rebind!!.from)
        assertEquals("${R.string.composer_errors_draft_attachments}", vm.mailUi.toasts.current!!.message)
        assertTrue(s.drafts.drafts.value.isEmpty())
        // Still unreadable: Save neither falls back to this phone nor closes.
        vm.saveDraft()
        assertNotNull(s.composer.current())
        assertTrue(s.drafts.drafts.value.isEmpty())
        return Triple(s, vm) { offline = false }
    }

    @Test
    fun attachmentsThatCantBeFoundOnTheNewCopyAreSettledBeforeTheNextSave() = runTest(main) {
        val (s, vm, online) = unsettledDraft()
        online()
        vm.saveDraft()
        assertNull(s.composer.current())
        assertTrue(s.drafts.drafts.value.isEmpty())
        val saved = s.mail.listMessages(MailView(ViewKind.DRAFTS)).single { it.subject == "Fwd: Call on Thursday" }
        assertEquals(listOf("Proposal-v3.pdf"), s.mail.fullMessage(saved.id).attachments.map { it.filename })
    }

    @Test
    fun attachmentsThatCantBeFoundOnTheNewCopyAreSettledBeforeSend() = runTest(main) {
        val (s, vm, online) = unsettledDraft()
        vm.send()
        // Not yet: nothing goes with an attachment Rukoo can't find.
        assertNotNull(s.composer.current())
        assertTrue(s.mail.listMessages(MailView(ViewKind.SENT)).none { it.subject == "Fwd: Call on Thursday" })
        online()
        vm.send()
        assertNull(s.composer.current())
        val sent = s.mail.listMessages(MailView(ViewKind.SENT)).first { it.subject == "Fwd: Call on Thursday" }
        assertEquals(listOf("Proposal-v3.pdf"), s.mail.fullMessage(sent.id).attachments.map { it.filename })
        assertTrue(s.mail.listMessages(MailView(ViewKind.DRAFTS)).none { it.subject == "Fwd: Call on Thursday" })
    }

    @Test
    fun sendAfterSettlingSendsWhatTheComposerHoldsThen() = runTest(main) {
        val preview = PreviewMailEngine(zone = ZoneId.of("Europe/Amsterdam"))
        var newCopy: String? = null
        var offline = true
        var gate: CompletableDeferred<Unit>? = null
        val mail = object : MailEngine by preview {
            override suspend fun saveDraft(message: OutgoingMessage): String? = preview.saveDraft(message).also { newCopy = it }

            override suspend fun fullMessage(id: String): FullMessage {
                if (id == newCopy) {
                    if (offline) throw MailException("offline")
                    gate?.await()
                }
                return preview.fullMessage(id)
            }
        }
        val (s, vm) = setup(preview = preview, mail = mail)
        val sanne = s.mail.listMessages(MailView.AllInboxes).first { it.subject == "Call on Thursday" }
        val forward = s.mail.replyFields(sanne.id, ReplyKind.FORWARD).toComposerState().copy(to = listOf(Address("", "joris@example.com")))
        val stored = s.mail.saveDraft(forward.toOutgoing("demo", Address("Demo User", "demo@example.com")))!!
        vm.editDraft(stored)
        s.composer.update { it.copy(body = "FYI", dirty = true) }
        vm.saveDraft()
        assertNotNull(s.composer.current()!!.rebind)
        // Back online, but slow: while Send waits for the copy, the user takes the attachment off.
        offline = false
        gate = CompletableDeferred()
        vm.send()
        s.composer.update { it.copy(attachments = emptyList(), dirty = true) }
        gate!!.complete(Unit)
        assertNull(s.composer.current())
        val sent = s.mail.listMessages(MailView(ViewKind.SENT)).first { it.subject == "Fwd: Call on Thursday" }
        assertTrue(s.mail.fullMessage(sent.id).attachments.isEmpty())
    }

    @Test
    fun aMailtoBeforeTheAccountsAreReadWaitsForThem() = runTest(main) {
        val preview = PreviewMailEngine(zone = ZoneId.of("Europe/Amsterdam"))
        val loaded = MutableStateFlow(false)
        val accounts = MutableStateFlow(emptyList<Account>())
        val mail = object : MailEngine by preview {
            override val loaded = loaded
            override val accounts = accounts
        }
        val (s, vm) = setup(preview = preview, mail = mail)
        vm.newMessage(Mailto.parse("mailto:help@x.com?subject=Bug"))
        assertNull(s.composer.current())
        // However long the accounts take, the request waits.
        advanceTimeBy(60_000)
        assertNull(s.composer.current())
        accounts.value = preview.accounts.value
        loaded.value = true
        val d = s.composer.current()!!
        assertEquals("demo", d.accountId)
        assertEquals("Bug", d.subject)
        assertEquals(listOf("help@x.com"), d.to.map { it.address })
    }

    private fun <T> assertNotNullAndGet(value: T?): T {
        assertNotNull(value)
        return value!!
    }
}
