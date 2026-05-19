package com.resolo.packet

import android.app.Activity
import android.app.Dialog
import android.view.View
import android.view.ViewGroup

fun Activity.forceLeftToRightLayout() {
    window.decorView.forceLeftToRightTree()
}

fun Dialog.forceLeftToRightLayout() {
    window?.decorView?.forceLeftToRightTree()
}

fun View.forceLeftToRightTree() {
    layoutDirection = View.LAYOUT_DIRECTION_LTR
    textDirection = View.TEXT_DIRECTION_LTR
    if (this is ViewGroup) {
        for (index in 0 until childCount) {
            getChildAt(index).forceLeftToRightTree()
        }
    }
}
