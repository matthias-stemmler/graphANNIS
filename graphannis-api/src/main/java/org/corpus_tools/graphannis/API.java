// Targeted by JavaCPP version 1.2.4: DO NOT EDIT THIS FILE

package org.corpus_tools.graphannis;

import java.nio.*;
import org.bytedeco.javacpp.*;
import org.bytedeco.javacpp.annotation.*;

public class API extends org.corpus_tools.graphannis.info.AnnisApiInfo {
    static { Loader.load(); }

@Name("std::vector<std::string>") public static class StringVector extends Pointer {
    static { Loader.load(); }
    /** Pointer cast constructor. Invokes {@link Pointer#Pointer(Pointer)}. */
    public StringVector(Pointer p) { super(p); }
    public StringVector(BytePointer ... array) { this(array.length); put(array); }
    public StringVector(String ... array) { this(array.length); put(array); }
    public StringVector()       { allocate();  }
    public StringVector(long n) { allocate(n); }
    private native void allocate();
    private native void allocate(@Cast("size_t") long n);
    public native @Name("operator=") @ByRef StringVector put(@ByRef StringVector x);

    public native long size();
    public native void resize(@Cast("size_t") long n);

    @Index public native @StdString BytePointer get(@Cast("size_t") long i);
    public native StringVector put(@Cast("size_t") long i, BytePointer value);
    @ValueSetter @Index public native StringVector put(@Cast("size_t") long i, @StdString String value);

    public StringVector put(BytePointer ... array) {
        if (size() != array.length) { resize(array.length); }
        for (int i = 0; i < array.length; i++) {
            put(i, array[i]);
        }
        return this;
    }

    public StringVector put(String ... array) {
        if (size() != array.length) { resize(array.length); }
        for (int i = 0; i < array.length; i++) {
            put(i, array[i]);
        }
        return this;
    }
}

@Name("std::vector<annis::api::UpdateEvent>") public static class UpdateEventList extends Pointer {
    static { Loader.load(); }
    /** Pointer cast constructor. Invokes {@link Pointer#Pointer(Pointer)}. */
    public UpdateEventList(Pointer p) { super(p); }
    public UpdateEventList(UpdateEvent ... array) { this(array.length); put(array); }
    public UpdateEventList()       { allocate();  }
    public UpdateEventList(long n) { allocate(n); }
    private native void allocate();
    private native void allocate(@Cast("size_t") long n);
    public native @Name("operator=") @ByRef UpdateEventList put(@ByRef UpdateEventList x);

    public native long size();
    public native void resize(@Cast("size_t") long n);

    @Index public native @ByRef UpdateEvent get(@Cast("size_t") long i);
    public native UpdateEventList put(@Cast("size_t") long i, UpdateEvent value);

    public UpdateEventList put(UpdateEvent ... array) {
        if (size() != array.length) { resize(array.length); }
        for (int i = 0; i < array.length; i++) {
            put(i, array[i]);
        }
        return this;
    }
}

// Parsed from annis/api/corpusstorage.h

// #pragma once

// #include <memory>
// #include <vector>
// #include <list>

// #include <annis/db.h>
// #include <annis/dbcache.h>
// #include <annis/json/jsonqueryparser.h>

// #include <annis/api/graphupdate.h>
/**
 * An API for managing corpora stored in a common location on the file system.
 */
@Namespace("annis::api") @NoOffset public static class CorpusStorage extends Pointer {
    static { Loader.load(); }
    /** Pointer cast constructor. Invokes {@link Pointer#Pointer(Pointer)}. */
    public CorpusStorage(Pointer p) { super(p); }


  public static class CountResult extends Pointer {
      static { Loader.load(); }
      /** Default native constructor. */
      public CountResult() { super((Pointer)null); allocate(); }
      /** Native array allocator. Access with {@link Pointer#position(long)}. */
      public CountResult(long size) { super((Pointer)null); allocateArray(size); }
      /** Pointer cast constructor. Invokes {@link Pointer#Pointer(Pointer)}. */
      public CountResult(Pointer p) { super(p); }
      private native void allocate();
      private native void allocateArray(long size);
      @Override public CountResult position(long position) {
          return (CountResult)super.position(position);
      }
  
    public native long matchCount(); public native CountResult matchCount(long matchCount);
    public native long documentCount(); public native CountResult documentCount(long documentCount);
  }

  public CorpusStorage(@StdString BytePointer databaseDir) { super((Pointer)null); allocate(databaseDir); }
  private native void allocate(@StdString BytePointer databaseDir);
  public CorpusStorage(@StdString String databaseDir) { super((Pointer)null); allocate(databaseDir); }
  private native void allocate(@StdString String databaseDir);

  /**
   * Count all occurrences of an AQL query in a single corpus.
   *
   * @param corpus
   * @param queryAsJSON
   * @return
   */
  public native long count(@ByVal StringVector corpora,
                    @StdString BytePointer queryAsJSON);
  public native long count(@ByVal StringVector corpora,
                    @StdString String queryAsJSON);


  /**
   * Count all occurrences of an AQL query in a single corpus.
   *
   * @param corpus
   * @param queryAsJSON
   * @return
   */
  public native @ByVal CountResult countExtra(@ByVal StringVector corpora,
                    @StdString BytePointer queryAsJSON);
  public native @ByVal CountResult countExtra(@ByVal StringVector corpora,
                    @StdString String queryAsJSON);


  /**
   * Find occurrences of an AQL query in a single corpus.
   * @param corpora
   * @param queryAsJSON
   * @param offset
   * @param limit
   * @return
   */
  public native @ByVal StringVector find(@ByVal StringVector corpora, @StdString BytePointer queryAsJSON, long offset/*=0*/,
                                  long limit/*=10*/);
  public native @ByVal StringVector find(@ByVal StringVector corpora, @StdString BytePointer queryAsJSON);
  public native @ByVal StringVector find(@ByVal StringVector corpora, @StdString String queryAsJSON, long offset/*=0*/,
                                  long limit/*=10*/);
  public native @ByVal StringVector find(@ByVal StringVector corpora, @StdString String queryAsJSON);

  public native void applyUpdate(@StdString BytePointer corpus, @Const @ByRef GraphUpdate update);
  public native void applyUpdate(@StdString String corpus, @Const @ByRef GraphUpdate update);
}

 // end namespace annis


// Parsed from annis/api/admin.h

// #pragma once

// #include <string>
  @Namespace("annis::api") public static class Admin extends Pointer {
      static { Loader.load(); }
      /** Pointer cast constructor. Invokes {@link Pointer#Pointer(Pointer)}. */
      public Admin(Pointer p) { super(p); }
      /** Native array allocator. Access with {@link Pointer#position(long)}. */
      public Admin(long size) { super((Pointer)null); allocateArray(size); }
      private native void allocateArray(long size);
      @Override public Admin position(long position) {
          return (Admin)super.position(position);
      }
  
    public Admin() { super((Pointer)null); allocate(); }
    private native void allocate();

    /**
    * Imports data in the relANNIS format to the internal format used by graphANNIS.
    * @param sourceFolder
    * @param targetFolder
    */
   public static native void importRelANNIS(@StdString BytePointer sourceFolder, @StdString BytePointer targetFolder);
   public static native void importRelANNIS(@StdString String sourceFolder, @StdString String targetFolder);
  }
 // end namespace annis::api


// Parsed from annis/api/graphupdate.h

// #pragma once

// #include <string>
// #include <memory>

// #include <vector>
// #include <string>

// #include <cereal/types/string.hpp>
// #include <cereal/types/list.hpp>

/** enum annis::api::UpdateEventType */
public static final int
  add_node = 0, delete_node = 1, add_node_label = 2, delete_node_label = 3;

@Namespace("annis::api") public static class UpdateEvent extends Pointer {
    static { Loader.load(); }
    /** Default native constructor. */
    public UpdateEvent() { super((Pointer)null); allocate(); }
    /** Native array allocator. Access with {@link Pointer#position(long)}. */
    public UpdateEvent(long size) { super((Pointer)null); allocateArray(size); }
    /** Pointer cast constructor. Invokes {@link Pointer#Pointer(Pointer)}. */
    public UpdateEvent(Pointer p) { super(p); }
    private native void allocate();
    private native void allocateArray(long size);
    @Override public UpdateEvent position(long position) {
        return (UpdateEvent)super.position(position);
    }

  public native long changeID(); public native UpdateEvent changeID(long changeID);
  public native @Cast("annis::api::UpdateEventType") int type(); public native UpdateEvent type(int type);
  public native @StdString BytePointer arg0(); public native UpdateEvent arg0(BytePointer arg0);
  public native @StdString BytePointer arg1(); public native UpdateEvent arg1(BytePointer arg1);
  public native @StdString BytePointer arg2(); public native UpdateEvent arg2(BytePointer arg2);
  public native @StdString BytePointer arg3(); public native UpdateEvent arg3(BytePointer arg3);
}

/**
 * \brief Lists updated that can be performed on a graph.
 *
 * This class is intended to make atomical updates to a graph (as represented by
 * the \class DB class possible.
 */
@Namespace("annis::api") @NoOffset public static class GraphUpdate extends Pointer {
    static { Loader.load(); }
    /** Pointer cast constructor. Invokes {@link Pointer#Pointer(Pointer)}. */
    public GraphUpdate(Pointer p) { super(p); }
    /** Native array allocator. Access with {@link Pointer#position(long)}. */
    public GraphUpdate(long size) { super((Pointer)null); allocateArray(size); }
    private native void allocateArray(long size);
    @Override public GraphUpdate position(long position) {
        return (GraphUpdate)super.position(position);
    }

  public GraphUpdate() { super((Pointer)null); allocate(); }
  private native void allocate();

  /**
   * \brief Adds an empty node with the given name to the graph.
   * If an node with this name already exists, nothing is done.
   *
   * @param name
   */
  public native void addNode(@StdString BytePointer name);
  public native void addNode(@StdString String name);

  /**
   * \brief Delete a node with the give name from the graph.
   *
   * This will delete all node labels as well. If this node does not exist, nothing is done.
   * @param name
   */
  public native void deleteNode(@StdString BytePointer name);
  public native void deleteNode(@StdString String name);

  /**
   * \brief Adds a label to an existing node.
   *
   * If the node does not exists or there is already a label with the same namespace and name, nothing is done.
   *
   * @param nodeName
   * @param ns The namespace of the label
   * @param name
   * @param value
   */
  public native void addNodeLabel(@StdString BytePointer nodeName, @StdString BytePointer ns, @StdString BytePointer name, @StdString BytePointer value);
  public native void addNodeLabel(@StdString String nodeName, @StdString String ns, @StdString String name, @StdString String value);

  /**
   * \brief Delete an existing label from a node.
   *
   * If the node or the label does not exist, nothing is done.
   *
   * @param nodeName
   * @param ns
   * @param name
   */
  public native void deleteNodeLabel(@StdString BytePointer nodeName, @StdString BytePointer ns, @StdString BytePointer name);
  public native void deleteNodeLabel(@StdString String nodeName, @StdString String ns, @StdString String name);

  /**
   * \brief Mark the current state as consistent.
   */
  public native void finish();

  public native @Const @ByRef UpdateEventList getDiffs();

  
}






}
