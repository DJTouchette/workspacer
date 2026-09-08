// Shared Formspree enhancement for the inline pilot form and the site-wide enquiry dialog.
// Every trigger has a real link to the HTML form if JavaScript is unavailable.
(function () {
  window.formspree =
    window.formspree ||
    function () {
      (window.formspree.q = window.formspree.q || []).push(arguments);
    };
  function focusFeedback(form, element) {
    var dialog = form.closest("dialog");
    if (element && (!dialog || dialog.open)) element.focus();
  }
  function enhance(form) {
    window.formspree("initForm", {
      formElement: form,
      formId: new URL(form.action).pathname.split("/").pop(),
      useDefaultStyles: false,
      onSubmit: function (context) {
        context.form.querySelector("[data-fs-submit-btn]").textContent =
          "Sending…";
      },
      onSuccess: function (context) {
        context.form.reset();
        focusFeedback(
          context.form,
          context.form.querySelector("[data-fs-success]"),
        );
      },
      onError: function (context) {
        focusFeedback(
          context.form,
          context.form.querySelector('[aria-invalid="true"]') ||
            context.form.querySelector('[data-fs-error=""]'),
        );
      },
      onFailure: function (context) {
        focusFeedback(
          context.form,
          context.form.querySelector('[data-fs-error=""]'),
        );
      },
    });
  }
  var inline = document.getElementById("pilot-form");
  if (inline) enhance(inline);
  var triggers = Array.from(document.querySelectorAll("[data-contact-open]"));
  if (
    !triggers.length ||
    typeof HTMLDialogElement === "undefined" ||
    typeof HTMLDialogElement.prototype.showModal !== "function"
  )
    return;

  var dialog = document.createElement("dialog");
  dialog.id = "contact-dialog";
  dialog.className = "wks-contact wks-contact-dialog";
  dialog.setAttribute("aria-labelledby", "contact-offer-title");
  dialog.innerHTML =
    '<div class="contact-heading"><h2 id="contact-offer-title">Let’s talk about Workspacer.</h2>' +
    '<button type="button" class="contact-close" aria-label="Close enquiry form">×</button></div>' +
    '<p class="contact-intro">Ask about a workflow, setup, or a guided pilot. I’ll reply by email.</p>' +
    `<form id="contact-form" class="contact-form wks-contact ph-no-capture" action="https://formspree.io/f/xbgjzbna" method="POST" aria-labelledby="contact-offer-title">
        <input type="hidden" name="subject" value="Workspacer enquiry" />
        <input type="hidden" name="source" value="Workspacer enterprise page" />
        <input type="text" name="_gotcha" tabindex="-1" autocomplete="off" aria-hidden="true" hidden />
        <div class="form-notice" id="contact-success" data-fs-success role="status" tabindex="-1">Thanks! Your enquiry has been submitted. I'll reply by email.</div>
        <div class="form-notice" id="contact-error" data-fs-error role="alert" tabindex="-1"></div>
        <div class="form-row">
          <div class="form-field">
            <label for="contact-name">Name</label>
            <input id="contact-name" name="name" type="text" autocomplete="name" maxlength="100" required data-fs-field aria-describedby="contact-name-error" />
            <span id="contact-name-error" data-fs-error="name"></span>
          </div>
          <div class="form-field">
            <label for="contact-email">Email</label>
            <input id="contact-email" name="email" type="email" autocomplete="email" maxlength="254" required data-fs-field aria-describedby="contact-email-error" />
            <span id="contact-email-error" data-fs-error="email"></span>
          </div>
        </div>
        <div class="form-field">
          <label for="contact-company">Company <span>(optional)</span></label>
          <input id="contact-company" name="company" type="text" autocomplete="organization" maxlength="160" data-fs-field aria-describedby="contact-company-error" />
          <span id="contact-company-error" data-fs-error="company"></span>
        </div>
        <div class="form-field">
          <label for="contact-message">What would you like help with?</label>
          <textarea id="contact-message" name="message" rows="5" maxlength="5000" required data-fs-field aria-describedby="contact-message-error" placeholder="Your codebase, the coding tools you use, and a task you'd like to try."></textarea>
          <span id="contact-message-error" data-fs-error="message"></span>
        </div>
        <p class="pilot-terms">For paid pilots, scope, price and support are agreed before we start. The software remains free and open source.</p>
        <button class="btn btn-fill" id="contact-submit" type="submit" data-fs-submit-btn>Send enquiry</button>
        <p class="form-note">Your enquiry is sent through Formspree so I can reply.</p>
      </form>` +
    '<p class="pilot-contact">Prefer email? <a href="mailto:thetouchstonedev@gmail.com?subject=Workspacer%20enquiry">thetouchstonedev@gmail.com</a></p>';
  document.body.appendChild(dialog);
  var form = dialog.querySelector("form");
  form.elements.source.value = document.title;
  form.elements.source.defaultValue = document.title;
  enhance(form);
  var opener;
  triggers.forEach(function (link) {
    link.setAttribute("aria-haspopup", "dialog");
    link.addEventListener("click", function (event) {
      if (
        event.ctrlKey ||
        event.metaKey ||
        event.shiftKey ||
        event.altKey ||
        event.button !== 0
      )
        return;
      event.preventDefault();
      opener = link;
      dialog.showModal();
      document.documentElement.classList.add("wks-contact-open");
      (
        form.querySelector("[data-fs-success][data-fs-active]") ||
        form.elements.name
      ).focus();
    });
  });
  dialog.querySelector(".contact-close").addEventListener("click", function () {
    dialog.close();
  });
  dialog.addEventListener("click", function (event) {
    if (event.target !== dialog) return;
    var rect = dialog.getBoundingClientRect();
    if (
      event.clientX < rect.left ||
      event.clientX > rect.right ||
      event.clientY < rect.top ||
      event.clientY > rect.bottom
    )
      dialog.close();
  });
  dialog.addEventListener("close", function () {
    document.documentElement.classList.remove("wks-contact-open");
    if (opener) opener.focus({ preventScroll: true });
  });
})();
